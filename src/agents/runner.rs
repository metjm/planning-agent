//! Shared agent execution runner.
//!
//! This module provides a unified process spawning, I/O handling, and timeout
//! management layer for all agent types (Claude, Codex, Gemini).

use crate::agents::log::AgentLogger;
use crate::agents::protocol::{AgentEvent, AgentOutput, AgentStreamParser};
use crate::agents::{AgentContext, AgentResult};
use crate::session_logger::SessionLogger;
use crate::tui::{CliInstanceId, TokenUsage, ToolResultSummary};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::time::Instant;

/// Default timeout for activity (no output) before killing the process.
pub const DEFAULT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(1800); // 30 minutes

/// Default overall timeout for the entire agent execution.
pub const DEFAULT_OVERALL_TIMEOUT: Duration = Duration::from_secs(21600); // 6 hours

/// Timeout for waiting for the process to exit after streams close.
pub const PROCESS_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimum interval between activity event emissions per CLI instance.
/// This prevents flooding the event loop with activity updates while
/// still maintaining accurate idle time tracking in the UI.
pub const ACTIVITY_EMIT_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Configuration for the agent runner.
#[derive(Clone)]
pub struct RunnerConfig {
    /// Agent name for logging
    pub agent_name: String,
    /// Working directory for the process
    pub working_dir: PathBuf,
    /// Timeout for inactivity
    pub activity_timeout: Duration,
    /// Overall execution timeout
    pub overall_timeout: Duration,
    /// Session logger for agent output
    pub session_logger: Option<Arc<SessionLogger>>,
    /// Optional cancellation signal receiver.
    /// When the sender sends `true`, the agent process will be killed.
    pub cancel_rx: Option<watch::Receiver<bool>>,
}

impl std::fmt::Debug for RunnerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerConfig")
            .field("agent_name", &self.agent_name)
            .field("working_dir", &self.working_dir)
            .field("activity_timeout", &self.activity_timeout)
            .field("overall_timeout", &self.overall_timeout)
            .field("session_logger", &self.session_logger.is_some())
            .field("cancel_rx", &self.cancel_rx.is_some())
            .finish()
    }
}

impl RunnerConfig {
    pub fn new(agent_name: String, working_dir: PathBuf) -> Self {
        Self {
            agent_name,
            working_dir,
            activity_timeout: DEFAULT_ACTIVITY_TIMEOUT,
            overall_timeout: DEFAULT_OVERALL_TIMEOUT,
            session_logger: None,
            cancel_rx: None,
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

    pub fn with_session_logger(mut self, logger: Arc<SessionLogger>) -> Self {
        self.session_logger = Some(logger);
        self
    }

    /// Sets a cancellation receiver for cooperative cancellation.
    /// When the sender sends `true`, the agent process will be killed.
    #[allow(dead_code)]
    pub fn with_cancel_rx(mut self, cancel_rx: watch::Receiver<bool>) -> Self {
        self.cancel_rx = Some(cancel_rx);
        self
    }
}

/// Trait for sending events during agent execution.
///
/// This abstracts over the event emission targets used by the runner.
pub trait EventEmitter: Send + Sync {
    fn send_output(&self, msg: String);
    fn send_streaming(&self, msg: String);
    fn send_bytes_received(&self, bytes: usize);
    fn send_turn_completed(&self);
    fn send_model_detected(&self, model: String);
    fn send_stop_reason(&self, reason: String);
    fn send_token_usage(&self, usage: TokenUsage);
    fn send_tool_started(
        &self,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
    );
    fn send_tool_finished(&self, tool_id: Option<String>);
    fn send_tool_result_received(
        &self,
        tool_id: Option<String>,
        is_error: bool,
        summary: ToolResultSummary,
    );
    fn send_agent_message(&self, msg: String);
    fn send_todos_update(&self, items: Vec<crate::tui::TodoItem>);

    // CLI instance lifecycle methods
    /// Allocate a new unique CLI instance ID.
    fn next_cli_instance_id(&self) -> CliInstanceId;
    /// Send a CLI instance started event.
    fn send_cli_instance_started(
        &self,
        id: CliInstanceId,
        pid: Option<u32>,
        started_at: std::time::Instant,
    );
    /// Send a CLI instance activity event.
    fn send_cli_instance_activity(&self, id: CliInstanceId, activity_at: std::time::Instant);
    /// Send a CLI instance finished event.
    fn send_cli_instance_finished(&self, id: CliInstanceId);
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
    fn send_tool_started(
        &self,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
    ) {
        self.context.session_sender.send_tool_started(
            self.context.phase.clone(),
            tool_id,
            display_name,
            input_preview,
            self.agent_name.clone(),
        );
    }
    fn send_tool_finished(&self, tool_id: Option<String>) {
        self.context
            .session_sender
            .send_tool_finished(tool_id, self.agent_name.clone());
    }
    fn send_tool_result_received(
        &self,
        tool_id: Option<String>,
        is_error: bool,
        summary: ToolResultSummary,
    ) {
        self.context.session_sender.send_tool_result_received(
            self.context.phase.clone(),
            tool_id,
            is_error,
            summary,
            self.agent_name.clone(),
        );
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
    fn next_cli_instance_id(&self) -> CliInstanceId {
        self.context.session_sender.next_cli_instance_id()
    }
    fn send_cli_instance_started(
        &self,
        id: CliInstanceId,
        pid: Option<u32>,
        started_at: std::time::Instant,
    ) {
        self.context.session_sender.send_cli_instance_started(
            id,
            self.agent_name.clone(),
            pid,
            started_at,
        );
    }
    fn send_cli_instance_activity(&self, id: CliInstanceId, activity_at: std::time::Instant) {
        self.context
            .session_sender
            .send_cli_instance_activity(id, activity_at);
    }
    fn send_cli_instance_finished(&self, id: CliInstanceId) {
        self.context.session_sender.send_cli_instance_finished(id);
    }
}

/// RAII guard that ensures CLI instance finished event is sent on drop.
struct CliInstanceGuard<'a> {
    id: CliInstanceId,
    emitter: &'a dyn EventEmitter,
    finished: bool,
}

impl<'a> CliInstanceGuard<'a> {
    fn new(id: CliInstanceId, emitter: &'a dyn EventEmitter) -> Self {
        Self {
            id,
            emitter,
            finished: false,
        }
    }

    /// Mark as finished (prevents double-emit on drop).
    fn finish(&mut self) {
        if !self.finished {
            self.emitter.send_cli_instance_finished(self.id);
            self.finished = true;
        }
    }
}

impl<'a> Drop for CliInstanceGuard<'a> {
    fn drop(&mut self) {
        self.finish();
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
            tool_use_id,
            ..
        } => {
            emitter.send_tool_started(tool_use_id, display_name.clone(), input_preview.clone());
        }
        AgentEvent::ToolResult {
            tool_use_id,
            is_error,
            content_lines,
            has_more,
        } => {
            // Normalize empty string to None for consistent handling
            let normalized_id = if tool_use_id.is_empty() {
                None
            } else {
                Some(tool_use_id.clone())
            };
            let summary = ToolResultSummary {
                first_line: content_lines.first().cloned().unwrap_or_default(),
                line_count: content_lines.len(),
                truncated: has_more,
            };
            emitter.send_tool_result_received(normalized_id.clone(), is_error, summary);
            emitter.send_tool_finished(normalized_id);
        }
        AgentEvent::TodosUpdate(items) => emitter.send_todos_update(items),
        AgentEvent::ContentBlockStart { name } => {
            emitter.send_tool_started(None, name.clone(), String::new());
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
        AgentEvent::ConversationIdCaptured(_) => {
            // Handled separately in run_agent_process for state capture
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
    let logger = config
        .session_logger
        .as_ref()
        .map(|sl| AgentLogger::new(&config.agent_name, sl.clone()));

    command.current_dir(&config.working_dir);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    emitter.send_output(format!("[agent:{}] Starting...", config.agent_name));

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to spawn {} process", config.agent_name))?;

    // Allocate CLI instance ID and emit started event
    let cli_instance_id = emitter.next_cli_instance_id();
    let pid = child.id();
    let std_started_at = std::time::Instant::now();
    emitter.send_cli_instance_started(cli_instance_id, pid, std_started_at);

    // Create RAII guard to ensure finished event is always emitted
    let mut _cli_guard = CliInstanceGuard::new(cli_instance_id, emitter);

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
    let mut captured_conversation_id: Option<String> = None;
    let mut last_stop_reason: Option<String> = None;

    let start_time = Instant::now();
    let mut last_activity = Instant::now();
    // Track last activity emit time for throttling
    let mut last_activity_emit = Instant::now();

    // Clone cancel_rx if present for use in select! loop
    let mut cancel_rx = config.cancel_rx.clone();

    loop {
        // Check overall timeout
        if start_time.elapsed() > config.overall_timeout {
            handle_overall_timeout(config, &logger, emitter, &mut child).await?;
        }

        let activity_deadline = last_activity + config.activity_timeout;

        tokio::select! {
            line = stdout_reader.next_line() => {
                last_activity = Instant::now();
                // Emit throttled activity event for CLI instance tracking
                if last_activity.duration_since(last_activity_emit) >= ACTIVITY_EMIT_MIN_INTERVAL {
                    last_activity_emit = last_activity;
                    emitter.send_cli_instance_activity(cli_instance_id, std::time::Instant::now());
                }
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
                                    // Handle special events
                                    match &event {
                                        AgentEvent::Result { output, cost, is_error: err } => {
                                            if let Some(out) = output {
                                                final_output = out.clone();
                                            }
                                            total_cost = *cost;
                                            is_error = *err;
                                        }
                                        AgentEvent::ConversationIdCaptured(id) => {
                                            captured_conversation_id = Some(id.clone());
                                            if let Some(ref logger) = logger {
                                                logger.log_line("conversation_id", id);
                                            }
                                        }
                                        AgentEvent::StopReason(reason) => {
                                            last_stop_reason = Some(reason.clone());
                                            emit_agent_event(event, emitter);
                                        }
                                        AgentEvent::TextContent(text) => {
                                            final_output.push_str(text);
                                            emit_agent_event(event, emitter);
                                        }
                                        _ => {
                                            emit_agent_event(event, emitter);
                                        }
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
                // Emit throttled activity event for CLI instance tracking
                if last_activity.duration_since(last_activity_emit) >= ACTIVITY_EMIT_MIN_INTERVAL {
                    last_activity_emit = last_activity;
                    emitter.send_cli_instance_activity(cli_instance_id, std::time::Instant::now());
                }
                if let Ok(Some(line)) = line {
                    if let Some(ref logger) = logger {
                        logger.log_line("stderr", &line);
                    }
                    emitter.send_streaming(format!("[stderr] {}", line));
                }
            }
            _ = tokio::time::sleep_until(activity_deadline) => {
                handle_activity_timeout(config, &logger, emitter, &mut child).await?;
            }
            _ = async {
                if let Some(ref mut rx) = cancel_rx {
                    // Wait for cancel signal
                    loop {
                        rx.changed().await.ok();
                        if *rx.borrow() {
                            return;
                        }
                    }
                } else {
                    // No cancel_rx, never resolves
                    std::future::pending::<()>().await
                }
            } => {
                // Cancellation requested
                if let Some(ref logger) = logger {
                    logger.log_line("cancelled", "cancellation signal received");
                }
                emitter.send_output(format!(
                    "[agent:{}] Cancellation requested, terminating...",
                    config.agent_name
                ));
                let _ = child.kill().await;
                // Set stop reason and return early with partial output
                last_stop_reason = Some("cancelled".to_string());
                is_error = false; // Cancellation is not an error
                break;
            }
        }
    }

    // Wait for process to exit (skip if cancelled - process already killed)
    let was_cancelled = last_stop_reason.as_deref() == Some("cancelled");
    if !was_cancelled {
        let status = wait_for_process(config, &logger, emitter, &mut child).await?;

        if let Some(ref logger) = logger {
            logger.log_line("exit", &format!("status: {}", status));
        }

        if !status.success() {
            is_error = true;
        }
    }

    if let Some(cost) = total_cost {
        emitter.send_output(format!("[agent:{}] Cost: ${:.4}", config.agent_name, cost));
    }

    if was_cancelled {
        emitter.send_output(format!("[agent:{}] Cancelled", config.agent_name));
    } else {
        emitter.send_output(format!("[agent:{}] Complete", config.agent_name));
    }

    Ok(AgentOutput {
        output: final_output,
        is_error,
        cost_usd: total_cost,
        conversation_id: captured_conversation_id,
        stop_reason: last_stop_reason,
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
            conversation_id: output.conversation_id,
            stop_reason: output.stop_reason,
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
            conversation_id: Some("conv-123".to_string()),
            stop_reason: Some("max_turns".to_string()),
        };
        let result: AgentResult = output.into();
        assert_eq!(result.output, "test output");
        assert!(!result.is_error);
        assert_eq!(result.cost_usd, Some(0.05));
        assert_eq!(result.conversation_id, Some("conv-123".to_string()));
        assert_eq!(result.stop_reason, Some("max_turns".to_string()));
    }

    #[test]
    fn test_agent_output_to_result_without_conversation_id() {
        let output = AgentOutput {
            output: "test output".to_string(),
            is_error: false,
            cost_usd: None,
            conversation_id: None,
            stop_reason: None,
        };
        let result: AgentResult = output.into();
        assert!(result.conversation_id.is_none());
        assert!(result.stop_reason.is_none());
    }
}
