use super::log::AgentLogger;
use super::{AgentContext, AgentResult};
use crate::config::AgentConfig;
use crate::tui::{Event, TokenUsage};
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

/// Extract the actual command from a bash command string
fn extract_bash_command(cmd: &str) -> String {
    let cmd = cmd.trim();

    // Split by command separators and take the first/main command
    let first_cmd = cmd
        .split("&&")
        .next()
        .unwrap_or(cmd)
        .split("||")
        .next()
        .unwrap_or(cmd)
        .split(';')
        .next()
        .unwrap_or(cmd)
        .split('|')
        .next()
        .unwrap_or(cmd)
        .trim();

    // Split into tokens and skip env vars (FOO=bar)
    let tokens: Vec<&str> = first_cmd.split_whitespace().collect();

    for token in tokens {
        // Skip env var assignments
        if token.contains('=') && !token.starts_with('-') {
            continue;
        }
        // Skip common prefixes
        if token == "sudo" || token == "env" || token == "time" || token == "nice" {
            continue;
        }
        // Extract command name from path
        let cmd_name = token.rsplit('/').next().unwrap_or(token);
        return format!("Bash:{}", cmd_name);
    }

    "Bash".to_string()
}

/// Claude Code CLI agent implementation
#[derive(Debug, Clone)]
pub struct ClaudeAgent {
    name: String,
    config: AgentConfig,
    working_dir: PathBuf,
    activity_timeout: Duration,
    overall_timeout: Duration,
}

impl ClaudeAgent {
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
        system_prompt: Option<String>,
        max_turns: Option<u32>,
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
            if let Some(ref sys_prompt) = system_prompt {
                logger.log_line(
                    "system_prompt",
                    &sys_prompt.chars().take(200).collect::<String>(),
                );
            }
        }

        let mut cmd = Command::new(&self.config.command);

        // Add base args from config
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Add prompt
        cmd.arg(&prompt);

        // Add system prompt if provided
        if let Some(ref sys_prompt) = system_prompt {
            cmd.arg("--append-system-prompt").arg(sys_prompt);
        }

        // Add allowed tools if configured
        if !self.config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.config.allowed_tools.join(","));
        }

        // Add max turns if provided
        if let Some(turns) = max_turns {
            cmd.arg("--max-turns").arg(turns.to_string());
        }

        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let _ = output_tx.send(Event::Output(
            "[agent:claude] Starting...".to_string(),
        ));

        let mut child = cmd
            .spawn()
            .context("Failed to spawn claude process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_result: Option<String> = None;
        let mut total_cost: Option<f64> = None;
        let mut is_error = false;
        let mut last_message_type: Option<String> = None;

        // Timeout tracking
        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        if let Some(ref logger) = logger {
            logger.log_line(
                "timeout",
                &format!(
                    "activity_timeout={:?}, overall_timeout={:?}",
                    self.activity_timeout, self.overall_timeout
                ),
            );
        }

        loop {
            // Check overall timeout
            if start_time.elapsed() > self.overall_timeout {
                if let Some(ref logger) = logger {
                    logger.log_line(
                        "timeout",
                        &format!(
                            "overall timeout triggered after {:?}",
                            start_time.elapsed()
                        ),
                    );
                }
                let _ = output_tx.send(Event::Output(format!(
                    "[agent:claude] ERROR: Exceeded overall timeout of {:?}",
                    self.overall_timeout
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Claude invocation exceeded overall timeout of {:?}",
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
                                if let Some(msg_type) = json.get("type").and_then(|v| v.as_str()) {
                                    match msg_type {
                                        "assistant" | "user" => {
                                            // Track turn transitions
                                            if msg_type == "assistant"
                                                && last_message_type.as_deref() == Some("user")
                                            {
                                                let _ = output_tx.send(Event::TurnCompleted);
                                            }
                                            last_message_type = Some(msg_type.to_string());

                                            if let Some(message) = json.get("message") {
                                                // Extract model name
                                                if let Some(model) =
                                                    message.get("model").and_then(|m| m.as_str())
                                                {
                                                    if !model.is_empty() {
                                                        let _ = output_tx.send(Event::ModelDetected(
                                                            model.to_string(),
                                                        ));
                                                    }
                                                }

                                                // Extract stop reason
                                                if let Some(stop) =
                                                    message.get("stop_reason").and_then(|s| s.as_str())
                                                {
                                                    let _ = output_tx
                                                        .send(Event::StopReason(stop.to_string()));
                                                }

                                                // Parse token usage
                                                if let Some(usage) = message.get("usage") {
                                                    let token_usage = TokenUsage {
                                                        input_tokens: usage
                                                            .get("input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        output_tokens: usage
                                                            .get("output_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        cache_creation_tokens: usage
                                                            .get("cache_creation_input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        cache_read_tokens: usage
                                                            .get("cache_read_input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                    };
                                                    let _ =
                                                        output_tx.send(Event::TokenUsage(token_usage));
                                                }

                                                if let Some(content) = message.get("content") {
                                                    if let Some(arr) = content.as_array() {
                                                        for item in arr {
                                                            // Handle text content
                                                            if let Some(text) =
                                                                item.get("text").and_then(|t| t.as_str())
                                                            {
                                                                for chunk in text.lines() {
                                                                    if !chunk.trim().is_empty() {
                                                                        let _ = output_tx.send(
                                                                            Event::Streaming(
                                                                                chunk.to_string(),
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                            // Handle tool_use content
                                                            if let Some(tool_type) = item
                                                                .get("type")
                                                                .and_then(|t| t.as_str())
                                                            {
                                                                if tool_type == "tool_use" {
                                                                    if let Some(name) = item
                                                                        .get("name")
                                                                        .and_then(|n| n.as_str())
                                                                    {
                                                                        let display_name = if name == "Bash"
                                                                        {
                                                                            item.get("input")
                                                                                .and_then(|i| i.get("command"))
                                                                                .and_then(|c| c.as_str())
                                                                                .map(extract_bash_command)
                                                                                .unwrap_or_else(|| {
                                                                                    "Bash".to_string()
                                                                                })
                                                                        } else {
                                                                            name.to_string()
                                                                        };

                                                                        let _ = output_tx.send(
                                                                            Event::ToolStarted(
                                                                                display_name.clone(),
                                                                            ),
                                                                        );

                                                                        let input_preview = item
                                                                            .get("input")
                                                                            .map(|i| {
                                                                                let s = i.to_string();
                                                                                let preview: String =
                                                                                    s.chars().take(100).collect();
                                                                                if s.len() > 100 {
                                                                                    format!("{}...", preview)
                                                                                } else {
                                                                                    preview
                                                                                }
                                                                            })
                                                                            .unwrap_or_default();
                                                                        let _ = output_tx.send(
                                                                            Event::Streaming(format!(
                                                                                "[Tool: {}] {}",
                                                                                display_name, input_preview
                                                                            )),
                                                                        );
                                                                    }
                                                                }
                                                                if tool_type == "tool_result" {
                                                                    let tool_use_id = item
                                                                        .get("tool_use_id")
                                                                        .and_then(|t| t.as_str())
                                                                        .unwrap_or("");
                                                                    let is_err = item
                                                                        .get("is_error")
                                                                        .and_then(|e| e.as_bool())
                                                                        .unwrap_or(false);
                                                                    let _ = output_tx.send(
                                                                        Event::ToolResultReceived {
                                                                            tool_id: tool_use_id.to_string(),
                                                                            is_error: is_err,
                                                                        },
                                                                    );
                                                                    let _ = output_tx.send(
                                                                        Event::ToolFinished(
                                                                            tool_use_id.to_string(),
                                                                        ),
                                                                    );

                                                                    if let Some(content) = item
                                                                        .get("content")
                                                                        .and_then(|c| c.as_str())
                                                                    {
                                                                        let lines: Vec<&str> =
                                                                            content.lines().take(5).collect();
                                                                        for (i, line) in lines.iter().enumerate()
                                                                        {
                                                                            let prefix = if i == 0 {
                                                                                "[Result] "
                                                                            } else {
                                                                                "         "
                                                                            };
                                                                            let _ = output_tx.send(
                                                                                Event::Streaming(format!(
                                                                                    "{}{}",
                                                                                    prefix, line
                                                                                )),
                                                                            );
                                                                        }
                                                                        if content.lines().count() > 5 {
                                                                            let _ = output_tx.send(
                                                                                Event::Streaming(
                                                                                    "         ...".to_string(),
                                                                                ),
                                                                            );
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_start" => {
                                            if let Some(content_block) = json.get("content_block") {
                                                if let Some(block_type) =
                                                    content_block.get("type").and_then(|t| t.as_str())
                                                {
                                                    if block_type == "tool_use" {
                                                        if let Some(name) = content_block
                                                            .get("name")
                                                            .and_then(|n| n.as_str())
                                                        {
                                                            let _ = output_tx.send(Event::ToolStarted(
                                                                name.to_string(),
                                                            ));
                                                            let _ = output_tx.send(Event::Streaming(
                                                                format!("[Tool: {}] starting...", name),
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_delta" => {
                                            if let Some(delta) = json.get("delta") {
                                                if let Some(text) =
                                                    delta.get("text").and_then(|t| t.as_str())
                                                {
                                                    for chunk in text.lines() {
                                                        if !chunk.trim().is_empty() {
                                                            let _ = output_tx.send(Event::Streaming(
                                                                chunk.to_string(),
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "result" => {
                                            final_result =
                                                json.get("result").and_then(|r| r.as_str()).map(Into::into);
                                            total_cost =
                                                json.get("total_cost_usd").and_then(|c| c.as_f64());
                                            is_error = json
                                                .get("is_error")
                                                .and_then(|e| e.as_bool())
                                                .unwrap_or(false);
                                        }
                                        _ => {}
                                    }
                                }
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
                    let inactivity_duration = last_activity.elapsed();
                    if let Some(ref logger) = logger {
                        logger.log_line(
                            "timeout",
                            &format!(
                                "activity timeout triggered after {:?} of inactivity",
                                inactivity_duration
                            ),
                        );
                    }
                    let _ = output_tx.send(Event::Output(format!(
                        "[agent:claude] WARNING: No activity for {:?}, terminating...",
                        self.activity_timeout
                    )));
                    let _ = child.kill().await;
                    anyhow::bail!(
                        "Claude subprocess became unresponsive (no output for {:?})",
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
                anyhow::bail!("Failed to wait for claude process: {}", e);
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
                    "[agent:claude] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                )));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Claude process did not exit within {:?} after stream closed",
                    PROCESS_WAIT_TIMEOUT
                );
            }
        };

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        if !status.success() {
            anyhow::bail!("Claude process exited with status {}", status);
        }

        if let Some(cost) = total_cost {
            let _ = output_tx.send(Event::Output(format!(
                "[agent:claude] Cost: ${:.4}",
                cost
            )));
        }

        Ok(AgentResult {
            output: final_result.unwrap_or_default(),
            is_error,
            cost_usd: total_cost,
        })
    }

    /// Execute with session-aware context for chat message routing
    pub async fn execute_streaming_with_context(
        &self,
        prompt: String,
        system_prompt: Option<String>,
        max_turns: Option<u32>,
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

        // Add base args from config
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Add prompt
        cmd.arg(&prompt);

        // Add system prompt if provided
        if let Some(ref sys_prompt) = system_prompt {
            cmd.arg("--append-system-prompt").arg(sys_prompt);
        }

        // Add allowed tools if configured
        if !self.config.allowed_tools.is_empty() {
            cmd.arg("--allowedTools")
                .arg(self.config.allowed_tools.join(","));
        }

        // Add max turns if provided
        if let Some(turns) = max_turns {
            cmd.arg("--max-turns").arg(turns.to_string());
        }

        cmd.current_dir(&self.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        context.session_sender.send_output(
            "[agent:claude] Starting...".to_string(),
        );

        let mut child = cmd
            .spawn()
            .context("Failed to spawn claude process")?;

        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let stderr = child.stderr.take().context("Failed to get stderr")?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut final_result: Option<String> = None;
        let mut total_cost: Option<f64> = None;
        let mut is_error = false;
        let mut last_message_type: Option<String> = None;

        // Timeout tracking
        let start_time = Instant::now();
        let mut last_activity = Instant::now();

        loop {
            // Check overall timeout
            if start_time.elapsed() > self.overall_timeout {
                context.session_sender.send_output(format!(
                    "[agent:claude] ERROR: Exceeded overall timeout of {:?}",
                    self.overall_timeout
                ));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Claude invocation exceeded overall timeout of {:?}",
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
                                if let Some(msg_type) = json.get("type").and_then(|v| v.as_str()) {
                                    match msg_type {
                                        "assistant" | "user" => {
                                            // Track turn transitions
                                            if msg_type == "assistant"
                                                && last_message_type.as_deref() == Some("user")
                                            {
                                                context.session_sender.send_turn_completed();
                                            }
                                            last_message_type = Some(msg_type.to_string());

                                            if let Some(message) = json.get("message") {
                                                // Extract model name
                                                if let Some(model) =
                                                    message.get("model").and_then(|m| m.as_str())
                                                {
                                                    if !model.is_empty() {
                                                        context.session_sender.send_model_detected(
                                                            model.to_string(),
                                                        );
                                                    }
                                                }

                                                // Extract stop reason
                                                if let Some(stop) =
                                                    message.get("stop_reason").and_then(|s| s.as_str())
                                                {
                                                    context.session_sender.send_stop_reason(stop.to_string());
                                                }

                                                // Parse token usage
                                                if let Some(usage) = message.get("usage") {
                                                    let token_usage = TokenUsage {
                                                        input_tokens: usage
                                                            .get("input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        output_tokens: usage
                                                            .get("output_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        cache_creation_tokens: usage
                                                            .get("cache_creation_input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                        cache_read_tokens: usage
                                                            .get("cache_read_input_tokens")
                                                            .and_then(|v| v.as_u64())
                                                            .unwrap_or(0),
                                                    };
                                                    context.session_sender.send_token_usage(token_usage);
                                                }

                                                if let Some(content) = message.get("content") {
                                                    if let Some(arr) = content.as_array() {
                                                        for item in arr {
                                                            // Handle text content - send to BOTH streaming and chat
                                                            if let Some(text) =
                                                                item.get("text").and_then(|t| t.as_str())
                                                            {
                                                                for chunk in text.lines() {
                                                                    if !chunk.trim().is_empty() {
                                                                        // Send to streaming panel (debug view)
                                                                        context.session_sender.send_streaming(
                                                                            chunk.to_string(),
                                                                        );
                                                                        // Send to chat panel
                                                                        context.session_sender.send_agent_message(
                                                                            self.name.clone(),
                                                                            context.phase.clone(),
                                                                            chunk.to_string(),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                            // Handle tool_use content - only streaming
                                                            if let Some(tool_type) = item
                                                                .get("type")
                                                                .and_then(|t| t.as_str())
                                                            {
                                                                if tool_type == "tool_use" {
                                                                    if let Some(name) = item
                                                                        .get("name")
                                                                        .and_then(|n| n.as_str())
                                                                    {
                                                                        let display_name = if name == "Bash"
                                                                        {
                                                                            item.get("input")
                                                                                .and_then(|i| i.get("command"))
                                                                                .and_then(|c| c.as_str())
                                                                                .map(extract_bash_command)
                                                                                .unwrap_or_else(|| {
                                                                                    "Bash".to_string()
                                                                                })
                                                                        } else {
                                                                            name.to_string()
                                                                        };

                                                                        context.session_sender.send_tool_started(
                                                                            display_name.clone(),
                                                                        );

                                                                        let input_preview = item
                                                                            .get("input")
                                                                            .map(|i| {
                                                                                let s = i.to_string();
                                                                                let preview: String =
                                                                                    s.chars().take(100).collect();
                                                                                if s.len() > 100 {
                                                                                    format!("{}...", preview)
                                                                                } else {
                                                                                    preview
                                                                                }
                                                                            })
                                                                            .unwrap_or_default();
                                                                        context.session_sender.send_streaming(format!(
                                                                            "[Tool: {}] {}",
                                                                            display_name, input_preview
                                                                        ));
                                                                    }
                                                                }
                                                                if tool_type == "tool_result" {
                                                                    let tool_use_id = item
                                                                        .get("tool_use_id")
                                                                        .and_then(|t| t.as_str())
                                                                        .unwrap_or("");
                                                                    let is_err = item
                                                                        .get("is_error")
                                                                        .and_then(|e| e.as_bool())
                                                                        .unwrap_or(false);
                                                                    context.session_sender.send_tool_result_received(
                                                                        tool_use_id.to_string(),
                                                                        is_err,
                                                                    );
                                                                    context.session_sender.send_tool_finished(
                                                                        tool_use_id.to_string(),
                                                                    );

                                                                    if let Some(content) = item
                                                                        .get("content")
                                                                        .and_then(|c| c.as_str())
                                                                    {
                                                                        let lines: Vec<&str> =
                                                                            content.lines().take(5).collect();
                                                                        for (i, line) in lines.iter().enumerate()
                                                                        {
                                                                            let prefix = if i == 0 {
                                                                                "[Result] "
                                                                            } else {
                                                                                "         "
                                                                            };
                                                                            context.session_sender.send_streaming(format!(
                                                                                "{}{}",
                                                                                prefix, line
                                                                            ));
                                                                        }
                                                                        if content.lines().count() > 5 {
                                                                            context.session_sender.send_streaming(
                                                                                "         ...".to_string(),
                                                                            );
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_start" => {
                                            if let Some(content_block) = json.get("content_block") {
                                                if let Some(block_type) =
                                                    content_block.get("type").and_then(|t| t.as_str())
                                                {
                                                    if block_type == "tool_use" {
                                                        if let Some(name) = content_block
                                                            .get("name")
                                                            .and_then(|n| n.as_str())
                                                        {
                                                            context.session_sender.send_tool_started(
                                                                name.to_string(),
                                                            );
                                                            context.session_sender.send_streaming(
                                                                format!("[Tool: {}] starting...", name),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "content_block_delta" => {
                                            // Route streaming text deltas to BOTH streaming and chat
                                            if let Some(delta) = json.get("delta") {
                                                if let Some(text) =
                                                    delta.get("text").and_then(|t| t.as_str())
                                                {
                                                    for chunk in text.lines() {
                                                        if !chunk.trim().is_empty() {
                                                            // Send to streaming panel
                                                            context.session_sender.send_streaming(
                                                                chunk.to_string(),
                                                            );
                                                            // Send to chat panel
                                                            context.session_sender.send_agent_message(
                                                                self.name.clone(),
                                                                context.phase.clone(),
                                                                chunk.to_string(),
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "result" => {
                                            final_result =
                                                json.get("result").and_then(|r| r.as_str()).map(Into::into);
                                            total_cost =
                                                json.get("total_cost_usd").and_then(|c| c.as_f64());
                                            is_error = json
                                                .get("is_error")
                                                .and_then(|e| e.as_bool())
                                                .unwrap_or(false);
                                        }
                                        _ => {}
                                    }
                                }
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
                        "[agent:claude] WARNING: No activity for {:?}, terminating...",
                        self.activity_timeout
                    ));
                    let _ = child.kill().await;
                    anyhow::bail!(
                        "Claude subprocess became unresponsive (no output for {:?})",
                        self.activity_timeout
                    );
                }
            }
        }

        // Wait for process exit with timeout
        let status = match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                anyhow::bail!("Failed to wait for claude process: {}", e);
            }
            Err(_) => {
                context.session_sender.send_output(format!(
                    "[agent:claude] WARNING: Process did not exit within {:?}, force killing...",
                    PROCESS_WAIT_TIMEOUT
                ));
                let _ = child.kill().await;
                anyhow::bail!(
                    "Claude process did not exit within {:?} after stream closed",
                    PROCESS_WAIT_TIMEOUT
                );
            }
        };

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        if !status.success() {
            anyhow::bail!("Claude process exited with status {}", status);
        }

        if let Some(cost) = total_cost {
            context.session_sender.send_output(format!(
                "[agent:claude] Cost: ${:.4}",
                cost
            ));
        }

        Ok(AgentResult {
            output: final_result.unwrap_or_default(),
            is_error,
            cost_usd: total_cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_bash_command() {
        assert_eq!(extract_bash_command("ls -la"), "Bash:ls");
        assert_eq!(extract_bash_command("cd /tmp && ls"), "Bash:cd");
        assert_eq!(extract_bash_command("FOO=bar npm install"), "Bash:npm");
        assert_eq!(extract_bash_command("sudo apt install"), "Bash:apt");
        assert_eq!(extract_bash_command("/usr/bin/python script.py"), "Bash:python");
    }

    #[test]
    fn test_claude_agent_new() {
        let config = AgentConfig {
            command: "claude".to_string(),
            args: vec!["-p".to_string()],
            allowed_tools: vec!["Read".to_string()],
        };
        let agent = ClaudeAgent::new("claude".to_string(), config, PathBuf::from("."));
        assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }
}
