//! Shared agent execution runner.
//!
//! This module provides a unified process spawning, I/O handling, and timeout
//! management layer for all agent types (Claude, Codex, Gemini).

use crate::agents::log::AgentLogger;
use crate::agents::protocol::{AgentEvent, AgentOutput, AgentStreamParser};
use crate::agents::{AgentContext, AgentResult};
use crate::tui::{Event, TokenUsage};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Default timeout for activity (no output) before killing the process.
pub const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(300);

/// Default overall timeout for the entire agent execution.
pub const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Timeout for waiting for the process to exit after streams close.
pub const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the agent runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Agent name for logging
    pub agent_name: String,
    /// Working directory for the process
    pub working_dir: PathBuf,
    /// Timeout for inactivity
    pub activity_timeout: Duration,
    /// Overall execution timeout
    pub overall_timeout: Duration,
}

impl RunnerConfig {
    pub fn new(agent_name: String, working_dir: PathBuf) -> Self {
        Self {
            agent_name,
            working_dir,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
        }
    }

    pub fn with_activity_timeout(mut self, timeout: Duration) -> Self {
        self.activity_timeout = timeout;
        self
    }

    pub fn with_overall_timeout(mut self, timeout: Duration) -> Self {
        self.overall_timeout = timeout;
        self
    }
}

/// Trait for sending events during agent execution.
///
/// This abstracts over the two modes of event sending:
/// - Legacy mode: sends to a raw `mpsc::UnboundedSender<Event>`
/// - Context mode: sends via `AgentContext` with session tracking
pub trait EventEmitter: Send + Sync {
    fn send_output(&self, msg: String);
    fn send_streaming(&self, msg: String);
    fn send_bytes_received(&self, bytes: usize);
    fn send_turn_completed(&self);
    fn send_model_detected(&self, model: String);
    fn send_stop_reason(&self, reason: String);
    fn send_token_usage(&self, usage: TokenUsage);
    fn send_tool_started(&self, name: String);
    fn send_tool_finished(&self, id: String);
    fn send_tool_result_received(&self, id: String, is_error: bool);
    fn send_agent_message(&self, msg: String);
    fn send_todos_update(&self, items: Vec<crate::tui::TodoItem>);
}

/// Event emitter for legacy mode (raw channel sender).
pub struct LegacyEmitter {
    tx: mpsc::UnboundedSender<Event>,
}

impl LegacyEmitter {
    pub fn new(tx: mpsc::UnboundedSender<Event>) -> Self {
        Self { tx }
    }
}

impl EventEmitter for LegacyEmitter {
    fn send_output(&self, msg: String) {
        let _ = self.tx.send(Event::Output(msg));
    }
    fn send_streaming(&self, msg: String) {
        let _ = self.tx.send(Event::Streaming(msg));
    }
    fn send_bytes_received(&self, bytes: usize) {
        let _ = self.tx.send(Event::BytesReceived(bytes));
    }
    fn send_turn_completed(&self) {
        let _ = self.tx.send(Event::TurnCompleted);
    }
    fn send_model_detected(&self, model: String) {
        let _ = self.tx.send(Event::ModelDetected(model));
    }
    fn send_stop_reason(&self, reason: String) {
        let _ = self.tx.send(Event::StopReason(reason));
    }
    fn send_token_usage(&self, usage: TokenUsage) {
        let _ = self.tx.send(Event::TokenUsage(usage));
    }
    fn send_tool_started(&self, name: String) {
        let _ = self.tx.send(Event::ToolStarted(name));
    }
    fn send_tool_finished(&self, id: String) {
        let _ = self.tx.send(Event::ToolFinished(id));
    }
    fn send_tool_result_received(&self, id: String, is_error: bool) {
        let _ = self.tx.send(Event::ToolResultReceived {
            tool_id: id,
            is_error,
        });
    }
    fn send_agent_message(&self, _msg: String) {
        // Legacy mode doesn't send agent messages
    }
    fn send_todos_update(&self, _items: Vec<crate::tui::TodoItem>) {
        // Legacy mode doesn't send todos updates
    }
}

/// Event emitter for context mode (session-aware).
pub struct ContextEmitter {
    context: AgentContext,
    agent_name: String,
}

impl ContextEmitter {
    pub fn new(context: AgentContext, agent_name: String) -> Self {
        Self {
            context,
            agent_name,
        }
    }
}

impl EventEmitter for ContextEmitter {
    fn send_output(&self, msg: String) {
        self.context.session_sender.send_output(msg);
    }
    fn send_streaming(&self, msg: String) {
        self.context.session_sender.send_streaming(msg);
    }
    fn send_bytes_received(&self, bytes: usize) {
        self.context.session_sender.send_bytes_received(bytes);
    }
    fn send_turn_completed(&self) {
        self.context.session_sender.send_turn_completed();
    }
    fn send_model_detected(&self, model: String) {
        self.context.session_sender.send_model_detected(model);
    }
    fn send_stop_reason(&self, reason: String) {
        self.context.session_sender.send_stop_reason(reason);
    }
    fn send_token_usage(&self, usage: TokenUsage) {
        self.context.session_sender.send_token_usage(usage);
    }
    fn send_tool_started(&self, name: String) {
        self.context.session_sender.send_tool_started(name);
    }
    fn send_tool_finished(&self, id: String) {
        self.context.session_sender.send_tool_finished(id);
    }
    fn send_tool_result_received(&self, id: String, is_error: bool) {
        self.context
            .session_sender
            .send_tool_result_received(id, is_error);
    }
    fn send_agent_message(&self, msg: String) {
        self.context.session_sender.send_agent_message(
            self.agent_name.clone(),
            self.context.phase.clone(),
            msg,
        );
    }
    fn send_todos_update(&self, items: Vec<crate::tui::TodoItem>) {
        self.context
            .session_sender
            .send_todos_update(self.agent_name.clone(), items);
    }
}

/// Convert AgentEvent to emitter calls.
fn emit_agent_event(event: AgentEvent, emitter: &dyn EventEmitter) {
    match event {
        AgentEvent::TurnCompleted => emitter.send_turn_completed(),
        AgentEvent::ModelDetected(model) => emitter.send_model_detected(model),
        AgentEvent::StopReason(reason) => emitter.send_stop_reason(reason),
        AgentEvent::TokenUsage(usage) => emitter.send_token_usage(usage.into()),
        AgentEvent::TextContent(text) => {
            emitter.send_streaming(text.clone());
            emitter.send_agent_message(text);
        }
        AgentEvent::ToolStarted {
            display_name,
            input_preview,
            ..
        } => {
            emitter.send_tool_started(display_name.clone());
            emitter.send_streaming(format!("[Tool: {}] {}", display_name, input_preview));
        }
        AgentEvent::ToolResult {
            tool_use_id,
            is_error,
            content_lines,
            has_more,
        } => {
            emitter.send_tool_result_received(tool_use_id.clone(), is_error);
            emitter.send_tool_finished(tool_use_id);
            for (i, line) in content_lines.iter().enumerate() {
                let prefix = if i == 0 { "[Result] " } else { "         " };
                emitter.send_streaming(format!("{}{}", prefix, line));
            }
            if has_more {
                emitter.send_streaming("         ...".to_string());
            }
        }
        AgentEvent::TodosUpdate(items) => emitter.send_todos_update(items),
        AgentEvent::ContentBlockStart { name } => {
            emitter.send_tool_started(name.clone());
            emitter.send_streaming(format!("[Tool: {}] starting...", name));
        }
        AgentEvent::ContentDelta(text) => {
            emitter.send_streaming(text.clone());
            emitter.send_agent_message(text);
        }
        AgentEvent::Result { .. } => {
            // Result events are handled separately for final output
        }
        AgentEvent::Error(msg) => {
            emitter.send_streaming(format!("[error] {}", msg));
        }
    }
}

/// Run an agent process with the given parser and emitter.
///
/// This function handles:
/// - Process spawning with proper I/O setup
/// - Stdout/stderr reading with line buffering
/// - Activity and overall timeout handling
/// - Event parsing and emission
/// - Graceful process termination
pub async fn run_agent_process<P: AgentStreamParser>(
    mut command: Command,
    config: &RunnerConfig,
    parser: &mut P,
    emitter: &dyn EventEmitter,
) -> Result<AgentOutput> {
    let logger = AgentLogger::new(&config.agent_name, &config.working_dir);

    command.current_dir(&config.working_dir);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    emitter.send_output(format!("[agent:{}] Starting...", config.agent_name));

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to spawn {} process", config.agent_name))?;

    let stdout = child
        .stdout
        .take()
        .context("Failed to get stdout from process")?;
    let stderr = child
        .stderr
        .take()
        .context("Failed to get stderr from process")?;

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let mut final_output = String::new();
    let mut total_cost: Option<f64> = None;
    let mut is_error = false;

    let start_time = Instant::now();
    let mut last_activity = Instant::now();

    loop {
        // Check overall timeout
        if start_time.elapsed() > config.overall_timeout {
            handle_overall_timeout(&config, &logger, emitter, &mut child).await?;
        }

        let activity_deadline = last_activity + config.activity_timeout;

        tokio::select! {
            line = stdout_reader.next_line() => {
                last_activity = Instant::now();
                match line {
                    Ok(Some(line)) => {
                        if let Some(ref logger) = logger {
                            logger.log_line("stdout", &line);
                        }
                        emitter.send_bytes_received(line.len());

                        // Parse the line and emit events
                        match parser.parse_line_multi(&line) {
                            Ok(events) => {
                                for event in events {
                                    // Handle Result events specially
                                    if let AgentEvent::Result { output, cost, is_error: err } = &event {
                                        if let Some(out) = output {
                                            final_output = out.clone();
                                        }
                                        total_cost = *cost;
                                        is_error = *err;
                                    } else {
                                        // Accumulate text content for final output
                                        if let AgentEvent::TextContent(ref text) = event {
                                            final_output.push_str(text);
                                        }
                                        emit_agent_event(event, emitter);
                                    }
                                }
                            }
                            Err(e) => {
                                if let Some(ref logger) = logger {
                                    logger.log_line("parse_error", &format!("{}", e));
                                }
                                // On parse error, still emit the raw line as text
                                emitter.send_streaming(line.clone());
                                final_output.push_str(&line);
                                final_output.push('\n');
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        emitter.send_output(format!("[error] Failed to read stdout: {}", e));
                        break;
                    }
                }
            }
            line = stderr_reader.next_line() => {
                last_activity = Instant::now();
                if let Ok(Some(line)) = line {
                    if let Some(ref logger) = logger {
                        logger.log_line("stderr", &line);
                    }
                    emitter.send_streaming(format!("[stderr] {}", line));
                }
            }
            _ = tokio::time::sleep_until(activity_deadline) => {
                handle_activity_timeout(&config, &logger, emitter, &mut child).await?;
            }
        }
    }

    // Wait for process to exit
    let status = wait_for_process(&config, &logger, emitter, &mut child).await?;

    if let Some(ref logger) = logger {
        logger.log_line("exit", &format!("status: {}", status));
    }

    if !status.success() {
        is_error = true;
    }

    if let Some(cost) = total_cost {
        emitter.send_output(format!("[agent:{}] Cost: ${:.4}", config.agent_name, cost));
    }

    emitter.send_output(format!("[agent:{}] Complete", config.agent_name));

    Ok(AgentOutput {
        output: final_output,
        is_error,
        cost_usd: total_cost,
    })
}

async fn handle_overall_timeout(
    config: &RunnerConfig,
    logger: &Option<AgentLogger>,
    emitter: &dyn EventEmitter,
    child: &mut Child,
) -> Result<()> {
    if let Some(ref logger) = logger {
        logger.log_line("timeout", "overall timeout triggered");
    }
    emitter.send_output(format!(
        "[agent:{}] ERROR: Exceeded overall timeout of {:?}",
        config.agent_name, config.overall_timeout
    ));
    let _ = child.kill().await;
    anyhow::bail!(
        "{} invocation exceeded overall timeout of {:?}",
        config.agent_name,
        config.overall_timeout
    );
}

async fn handle_activity_timeout(
    config: &RunnerConfig,
    logger: &Option<AgentLogger>,
    emitter: &dyn EventEmitter,
    child: &mut Child,
) -> Result<()> {
    if let Some(ref logger) = logger {
        logger.log_line("timeout", "activity timeout triggered");
    }
    emitter.send_output(format!(
        "[agent:{}] WARNING: No activity for {:?}, terminating...",
        config.agent_name, config.activity_timeout
    ));
    let _ = child.kill().await;
    anyhow::bail!(
        "{} subprocess became unresponsive (no output for {:?})",
        config.agent_name,
        config.activity_timeout
    );
}

async fn wait_for_process(
    config: &RunnerConfig,
    logger: &Option<AgentLogger>,
    emitter: &dyn EventEmitter,
    child: &mut Child,
) -> Result<std::process::ExitStatus> {
    match tokio::time::timeout(PROCESS_WAIT_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(e)) => {
            if let Some(ref logger) = logger {
                logger.log_line("timeout", &format!("failed to wait for process: {}", e));
            }
            anyhow::bail!("Failed to wait for {} process: {}", config.agent_name, e);
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
            emitter.send_output(format!(
                "[agent:{}] WARNING: Process did not exit within {:?}, force killing...",
                config.agent_name, PROCESS_WAIT_TIMEOUT
            ));
            let _ = child.kill().await;
            anyhow::bail!(
                "{} process did not exit within {:?} after stream closed",
                config.agent_name,
                PROCESS_WAIT_TIMEOUT
            );
        }
    }
}

/// Helper to convert AgentOutput to AgentResult for compatibility.
impl From<AgentOutput> for AgentResult {
    fn from(output: AgentOutput) -> Self {
        AgentResult {
            output: output.output,
            is_error: output.is_error,
            cost_usd: output.cost_usd,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_config_defaults() {
        let config = RunnerConfig::new("test".to_string(), PathBuf::from("."));
        assert_eq!(config.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
        assert_eq!(config.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
    }

    #[test]
    fn test_runner_config_custom_timeouts() {
        let config = RunnerConfig::new("test".to_string(), PathBuf::from("."))
            .with_activity_timeout(Duration::from_secs(60))
            .with_overall_timeout(Duration::from_secs(600));
        assert_eq!(config.activity_timeout, Duration::from_secs(60));
        assert_eq!(config.overall_timeout, Duration::from_secs(600));
    }

    #[test]
    fn test_agent_output_to_result() {
        let output = AgentOutput {
            output: "test output".to_string(),
            is_error: false,
            cost_usd: Some(0.05),
        };
        let result: AgentResult = output.into();
        assert_eq!(result.output, "test output");
        assert!(!result.is_error);
        assert_eq!(result.cost_usd, Some(0.05));
    }
}
