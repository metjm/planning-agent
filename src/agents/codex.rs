use super::log::AgentLogger;
use super::AgentResult;
use crate::config::AgentConfig;
use crate::tui::Event;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Default activity timeout: 5 minutes
const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);

/// Default overall timeout: 30 minutes
const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Timeout for waiting for process exit after streams close
const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenAI Codex CLI agent implementation
#[derive(Debug, Clone)]
pub struct CodexAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl CodexAgent {
    pub fn new(name: String, config: AgentConfig, working_dir: PathBuf) -> Self {
        Self {
            name,
            config,
            working_dir,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    #[allow(dead_code)]
    pub fn with_activity_timeout(mut self, timeout: Duration) -> Self {
        self.activity_timeout = timeout;
        self
    }

    #[allow(dead_code)]
    pub fn with_overall_timeout(mut self, timeout: Duration) -> Self {
        self.overall_timeout = timeout;
        self
    }

    /// Execute with streaming output
    pub async fn execute_streaming(
        &self,
        prompt: String,
        _system_prompt: Option<String>, // Codex doesn't support separate system prompt
        _max_turns: Option<u32>,
        output_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<AgentResult> {
        let logger = AgentLogger::new(&self.name, &self.working_dir);
        if let Some(ref logger) = logger {
            let args = if self.config.args.is_empty() {
                String::new()
            } else {
                format!(" {}", self.config.args.join(" "))
            };
            logger.log_line(
                "start",
                &format!("command: {}{}", self.config.command, args),
            );
            logger.log_line(
                "prompt",
                &prompt.chars().take(200).collect::<String>(),
            );
        }

        let mut cmd = Command::new(&self.config.command);

        // Add base args (typically ["exec", "--json"])
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Add prompt
        cmd.arg(&prompt);

        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let _ = output_tx.send(Event::Output("[agent:codex] Starting...".to_string()));

        let mut child = cmd.spawn().context("Failed to spawn codex process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_output = String::new();
        let mut is_error = false;

        // Timeout tracking
        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        loop {
            // Check overall timeout
            if start_time.elapsed() > self.overall_timeout {
                let _ = output_tx.send(Event::Output(format!(
                    "[agent:codex] ERROR: Exceeded overall timeout of {:?}",
                    self.overall_timeout
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Codex invocation exceeded overall timeout of {:?}",
                    self.overall_timeout
                );
            }

            let activity_deadline = last_activity + self.activity_timeout;

            tokio::select! {
                line = stdout_reader.next_line() => {
                    last_activity = Instant::now();
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref logger) = logger {
                                logger.log_line("stdout", &line);
                            }
                            let _ = output_tx.send(Event::BytesReceived(line.len()));

                            // Try to parse as JSON (Codex outputs NDJSON events)
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                // Codex outputs state change events
                                if let Some(event_type) = json.get("type").and_then(|v| v.as_str()) {
                                    match event_type {
                                        "message" | "content" | "text" => {
                                            if let Some(content) = json.get("content")
                                                .or_else(|| json.get("text"))
                                                .or_else(|| json.get("message"))
                                                .and_then(|c| c.as_str())
                                            {
                                                let _ = output_tx.send(Event::Streaming(content.to_string()));
                                                final_output.push_str(content);
                                            }
                                        }
                                        "function_call" | "tool_call" => {
                                            if let Some(name) = json.get("name")
                                                .or_else(|| json.get("function").and_then(|f| f.get("name")))
                                                .and_then(|n| n.as_str())
                                            {
                                                let _ = output_tx.send(Event::ToolStarted(name.to_string()));
                                                let _ = output_tx.send(Event::Streaming(
                                                    format!("[Tool: {}]", name)
                                                ));
                                            }
                                        }
                                        "function_result" | "tool_result" => {
                                            let tool_id = json.get("call_id")
                                                .or_else(|| json.get("id"))
                                                .and_then(|i| i.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let _ = output_tx.send(Event::ToolFinished(tool_id));
                                        }
                                        "done" | "complete" | "finished" => {
                                            // Final event - extract complete message
                                            if let Some(message) = json.get("message")
                                                .or_else(|| json.get("result"))
                                                .or_else(|| json.get("output"))
                                                .and_then(|m| m.as_str())
                                            {
                                                final_output = message.to_string();
                                            }
                                        }
                                        "error" => {
                                            is_error = true;
                                            if let Some(message) = json.get("message")
                                                .or_else(|| json.get("error"))
                                                .and_then(|m| m.as_str())
                                            {
                                                let _ = output_tx.send(Event::Streaming(
                                                    format!("[error] {}", message)
                                                ));
                                                final_output = message.to_string();
                                            }
                                        }
                                        _ => {
                                            // Stream any content we find
                                            if let Some(content) = json.get("content")
                                                .or_else(|| json.get("text"))
                                                .and_then(|c| c.as_str())
                                            {
                                                let _ = output_tx.send(Event::Streaming(content.to_string()));
                                                final_output.push_str(content);
                                            }
                                        }
                                    }
                                } else {
                                    // Not a typed event - try to extract content directly
                                    if let Some(content) = json.get("content")
                                        .or_else(|| json.get("text"))
                                        .or_else(|| json.get("output"))
                                        .and_then(|c| c.as_str())
                                    {
                                        let _ = output_tx.send(Event::Streaming(content.to_string()));
                                        final_output.push_str(content);
                                    }
                                }
                            } else {
                                // Plain text output
                                let _ = output_tx.send(Event::Streaming(line.clone()));
                                final_output.push_str(&line);
                                final_output.push('\n');
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = output_tx.send(Event::Output(format!(
                                "[error] Failed to read stdout: {}",
                                e
                            )));
                            break;
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    last_activity = Instant::now();
                    match line {
                        Ok(Some(line)) => {
                            if let Some(ref logger) = logger {
                                logger.log_line("stderr", &line);
                            }
                            let _ = output_tx.send(Event::Streaming(format!("[stderr] {}", line)));
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
                _ = tokio::time::sleep_until(activity_deadline) => {
                    if let Some(ref logger) = logger {
                        logger.log_line(
                            "timeout",
                            &format!(
                                "activity timeout triggered after {:?}",
                                self.activity_timeout
                            ),
                        );
                    }
                    let _ = output_tx.send(Event::Output(format!(
                        "[agent:codex] WARNING: No activity for {:?}, terminating...",
                        self.activity_timeout
                    )));
                    let _ = child.kill().await;
                    anyhow::bail!(
                        "Codex subprocess became unresponsive (no output for {:?})",
                        self.activity_timeout
                    );
                }
            }
        }

        // Wait for process exit with timeout
        let status = match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                if let Some(ref logger) = logger {
                    logger.log_line("timeout", &format!("failed to wait for process: {}", e));
                }
                anyhow::bail!("Failed to wait for codex process: {}", e);
            }
            Err(_) => {
                if let Some(ref logger) = logger {
                    logger.log_line(
                        "timeout",
                        &format!(
                            "process did not exit within {:?} after stream closed, force killing",
                            PROCESS_WAIT_TIMEOUT
                        ),
                    );
                }
                let _ = output_tx.send(Event::Output(format!(
                    "[agent:codex] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Codex process did not exit within {:?} after stream closed",
                    PROCESS_WAIT_TIMEOUT
                );
            }
        };

        if !status.success() {
            is_error = true;
        }

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        let _ = output_tx.send(Event::Output("[agent:codex] Complete".to_string()));

        Ok(AgentResult {
            output: final_output,
            is_error,
            cost_usd: None, // Codex CLI doesn't report cost
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_agent_new() {
        let config = AgentConfig {
            command: "codex".to_string(),
            args: vec!["exec".to_string(), "--json".to_string()],
            allowed_tools: vec![],
        };
        let agent = CodexAgent::new("codex".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
