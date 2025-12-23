use crate::agents::log::AgentLogger;
use crate::agents::{AgentContext, AgentResult};
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

const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);

const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct GeminiAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl GeminiAgent {
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

    #[allow(dead_code)] 
    pub async fn execute_streaming(
        &self,
        prompt: String,
        _system_prompt: Option<String>, 
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

        for arg in &self.config.args {
            cmd.arg(arg);
        }

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

        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        loop {

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
                            if let Some(ref logger) = logger {
                                logger.log_line("stdout", &line);
                            }
                            let _ = output_tx.send(Event::BytesReceived(line.len()));

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {

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

                                if let Some(function_call) = json.get("functionCall")
                                    .or_else(|| json.get("function_call"))
                                {
                                    if let Some(name) = function_call.get("name").and_then(|n| n.as_str()) {
                                        let _ = output_tx.send(Event::ToolStarted(name.to_string()));
                                        let _ = output_tx.send(Event::Streaming(format!("[Tool: {}]", name)));
                                    }
                                }

                                if let Some(function_response) = json.get("functionResponse")
                                    .or_else(|| json.get("function_response"))
                                {
                                    let id = function_response.get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let _ = output_tx.send(Event::ToolFinished(id));
                                }

                                if let Some(error) = json.get("error") {
                                    is_error = true;
                                    if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                                        let _ = output_tx.send(Event::Streaming(format!("[error] {}", message)));
                                        final_output = message.to_string();
                                    }
                                }
                            } else {

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

        let status = match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                if let Some(ref logger) = logger {
                    logger.log_line("timeout", &format!("failed to wait for process: {}", e));
                }
                anyhow::bail!("Failed to wait for gemini process: {}", e);
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

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        let _ = output_tx.send(Event::Output("[agent:gemini] Complete".to_string()));

        Ok(AgentResult {
            output: final_output,
            is_error,
            cost_usd: None, 
        })
    }

    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        _system_prompt: Option<String>,
        _max_turns: Option<u32>,
        context: AgentContext,
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
                &format!("command: {}{} (with context)", self.config.command, args),
            );
        }

        let mut cmd = Command::new(&self.config.command);

        for arg in &self.config.args {
            cmd.arg(arg);
        }

        cmd.arg(&prompt);
        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        context.session_sender.send_output("[agent:gemini] Starting...".to_string());

        let mut child = cmd.spawn().context("Failed to spawn gemini process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_output = String::new();
        let mut is_error = false;

        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        loop {
            if start_time.elapsed() > self.overall_timeout {
                context.session_sender.send_output(format!(
                    "[agent:gemini] ERROR: Exceeded overall timeout of {:?}",
                    self.overall_timeout
                ));
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
                            if let Some(ref logger) = logger {
                                logger.log_line("stdout", &line);
                            }
                            context.session_sender.send_bytes_received(line.len());

                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                if let Some(response) = json.get("response")
                                    .or_else(|| json.get("text"))
                                    .or_else(|| json.get("content"))
                                    .or_else(|| json.get("output"))
                                    .or_else(|| json.get("result"))
                                    .and_then(|r| r.as_str())
                                {
                                    context.session_sender.send_streaming(response.to_string());
                                    context.session_sender.send_agent_message(
                                        self.name.clone(),
                                        context.phase.clone(),
                                        response.to_string(),
                                    );
                                    final_output.push_str(response);
                                }

                                if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
                                    for candidate in candidates {
                                        if let Some(content) = candidate.get("content") {
                                            if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                                                for part in parts {
                                                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                                        context.session_sender.send_streaming(text.to_string());
                                                        context.session_sender.send_agent_message(
                                                            self.name.clone(),
                                                            context.phase.clone(),
                                                            text.to_string(),
                                                        );
                                                        final_output.push_str(text);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(function_call) = json.get("functionCall")
                                    .or_else(|| json.get("function_call"))
                                {
                                    if let Some(name) = function_call.get("name").and_then(|n| n.as_str()) {
                                        context.session_sender.send_tool_started(name.to_string());
                                        context.session_sender.send_streaming(format!("[Tool: {}]", name));
                                    }
                                }

                                if let Some(function_response) = json.get("functionResponse")
                                    .or_else(|| json.get("function_response"))
                                {
                                    let id = function_response.get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    context.session_sender.send_tool_finished(id);
                                }

                                if let Some(error) = json.get("error") {
                                    is_error = true;
                                    if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                                        context.session_sender.send_streaming(format!("[error] {}", message));
                                        final_output = message.to_string();
                                    }
                                }
                            } else {
                                context.session_sender.send_streaming(line.clone());
                                context.session_sender.send_agent_message(
                                    self.name.clone(),
                                    context.phase.clone(),
                                    line.clone(),
                                );
                                final_output.push_str(&line);
                                final_output.push('\n');
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            context.session_sender.send_output(format!(
                                "[error] Failed to read stdout: {}",
                                e
                            ));
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
                            context.session_sender.send_streaming(format!("[stderr] {}", line));
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
                _ = tokio::time::sleep_until(activity_deadline) => {
                    context.session_sender.send_output(format!(
                        "[agent:gemini] WARNING: No activity for {:?}, terminating...",
                        self.activity_timeout
                    ));
                    let _ = child.kill().await;
                    anyhow::bail!(
                        "Gemini subprocess became unresponsive (no output for {:?})",
                        self.activity_timeout
                    );
                }
            }
        }

        let status = match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                anyhow::bail!("Failed to wait for gemini process: {}", e);
            }
            Err(_) => {
                context.session_sender.send_output(format!(
                    "[agent:gemini] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                ));
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

        context.session_sender.send_output("[agent:gemini] Complete".to_string());

        Ok(AgentResult {
            output: final_output,
            is_error,
            cost_usd: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SessionPersistenceConfig;

    #[test]
    fn test_gemini_agent_new() {
        let config = AgentConfig {
            command: "gemini".to_string(),
            args: vec!["-p".to_string(), "--output-format".to_string(), "json".to_string()],
            allowed_tools: vec![],
            session_persistence: SessionPersistenceConfig::default(),
        };
        let agent = GeminiAgent::new("gemini".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
