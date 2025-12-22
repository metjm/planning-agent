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

/// Google Gemini CLI agent implementation
#[derive(Debug, Clone)]
pub struct GeminiAgent {
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl GeminiAgent {
    pub fn new(config: AgentConfig, working_dir: PathBuf) -> Self {
        Self {
            config,
            working_dir,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
        }
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
        _system_prompt: Option<String>, // Gemini CLI may use different mechanism
        _max_turns: Option<u32>,
        output_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<AgentResult> {
        let mut cmd = Command::new(&self.config.command);

        // Add base args from config
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Add prompt
        cmd.arg(&prompt);

        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let _ = output_tx.send(Event::Output("[agent:gemini] Starting...".to_string()));

        let mut child = cmd.spawn().context("Failed to spawn gemini process")?;

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
                    "[agent:gemini] ERROR: Exceeded overall timeout of {:?}",
                    self.overall_timeout
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Gemini invocation exceeded overall timeout of {:?}",
                    self.overall_timeout
                );
            }

            let activity_deadline = last_activity + self.activity_timeout;

            tokio::select! {
                line = stdout_reader.next_line() => {
                    last_activity = Instant::now();
                    match line {
                        Ok(Some(line)) => {
                            let _ = output_tx.send(Event::BytesReceived(line.len()));

                            // Try to parse as JSON (Gemini may output single JSON or NDJSON)
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                // Extract response content - Gemini may use various field names
                                if let Some(response) = json.get("response")
                                    .or_else(|| json.get("text"))
                                    .or_else(|| json.get("content"))
                                    .or_else(|| json.get("output"))
                                    .or_else(|| json.get("result"))
                                    .and_then(|r| r.as_str())
                                {
                                    let _ = output_tx.send(Event::Streaming(response.to_string()));
                                    final_output.push_str(response);
                                }

                                // Check for candidates array (common in Gemini responses)
                                if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
                                    for candidate in candidates {
                                        if let Some(content) = candidate.get("content") {
                                            if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                                                for part in parts {
                                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                                        let _ = output_tx.send(Event::Streaming(text.to_string()));
                                                        final_output.push_str(text);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Check for function calls
                                if let Some(function_call) = json.get("functionCall")
                                    .or_else(|| json.get("function_call"))
                                {
                                    if let Some(name) = function_call.get("name").and_then(|n| n.as_str()) {
                                        let _ = output_tx.send(Event::ToolStarted(name.to_string()));
                                        let _ = output_tx.send(Event::Streaming(format!("[Tool: {}]", name)));
                                    }
                                }

                                // Check for function response
                                if let Some(function_response) = json.get("functionResponse")
                                    .or_else(|| json.get("function_response"))
                                {
                                    let id = function_response.get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let _ = output_tx.send(Event::ToolFinished(id));
                                }

                                // Check for error
                                if let Some(error) = json.get("error") {
                                    is_error = true;
                                    if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                                        let _ = output_tx.send(Event::Streaming(format!("[error] {}", message)));
                                        final_output = message.to_string();
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
                            let _ = output_tx.send(Event::Streaming(format!("[stderr] {}", line)));
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
                _ = tokio::time::sleep_until(activity_deadline) => {
                    let _ = output_tx.send(Event::Output(format!(
                        "[agent:gemini] WARNING: No activity for {:?}, terminating...",
                        self.activity_timeout
                    )));
                    let _ = child.kill().await;
                    anyhow::bail!(
                        "Gemini subprocess became unresponsive (no output for {:?})",
                        self.activity_timeout
                    );
                }
            }
        }

        // Wait for process exit with timeout
        let status = match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                anyhow::bail!("Failed to wait for gemini process: {}", e);
            }
            Err(_) => {
                let _ = output_tx.send(Event::Output(format!(
                    "[agent:gemini] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Gemini process did not exit within {:?} after stream closed",
                    PROCESS_WAIT_TIMEOUT
                );
            }
        };

        if !status.success() {
            is_error = true;
        }

        let _ = output_tx.send(Event::Output("[agent:gemini] Complete".to_string()));

        Ok(AgentResult {
            output: final_output,
            is_error,
            cost_usd: None, // Gemini CLI doesn't report cost
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemini_agent_new() {
        let config = AgentConfig {
            command: "gemini".to_string(),
            args: vec!["-p".to_string(), "--output-format".to_string(), "json".to_string()],
            allowed_tools: vec![],
        };
        let agent = GeminiAgent::new(config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
