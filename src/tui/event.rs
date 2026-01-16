use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, KeyEventKind};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::cli_usage::AccountUsage;
use crate::session_logger::{LogCategory, LogLevel, SessionLogger};
use crate::state::State;
use crate::tui::file_index::FileIndex;
use crate::tui::session::TodoItem;
use crate::update::{UpdateResult, UpdateStatus, VersionInfo};
use std::sync::Arc;

/// Command sent from UI to workflow to control execution.
#[derive(Debug, Clone)]
pub enum WorkflowCommand {
    /// Interrupt the current workflow with user feedback.
    Interrupt { feedback: String },
    /// Stop the workflow cleanly at the next phase boundary.
    /// A snapshot will be saved for later resumption.
    Stop,
}

/// Custom error type for cancellation - avoids fragile string matching.
#[derive(Debug, Clone)]
pub struct CancellationError {
    pub feedback: String,
}

impl std::fmt::Display for CancellationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancelled by user interrupt: {}", self.feedback)
    }
}

impl std::error::Error for CancellationError {}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

/// Events for the TUI system. Some variants are for multi-session support
/// and may not be used in all code paths.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    Key(KeyEvent),
    Paste(String),
    Tick,
    Resize,

    Output(String),
    Streaming(String),
    ToolStarted {
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
    },
    ToolFinished { tool_id: Option<String>, agent_name: String },
    StateUpdate(State),
    RequestUserApproval(String),
    BytesReceived(usize),
    TokenUsage(TokenUsage),
    PhaseStarted(String),
    TurnCompleted,
    ModelDetected(String),
    ToolResultReceived { tool_id: Option<String>, is_error: bool, agent_name: String },
    StopReason(String),

    SessionOutput { session_id: usize, line: String },
    SessionStreaming { session_id: usize, line: String },
    SessionStateUpdate { session_id: usize, state: State },
    SessionApprovalRequest { session_id: usize, summary: String },
    SessionReviewDecisionRequest { session_id: usize, summary: String },
    SessionTokenUsage { session_id: usize, usage: TokenUsage },
    SessionToolStarted {
        session_id: usize,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
    },
    SessionToolFinished { session_id: usize, tool_id: Option<String>, agent_name: String },
    SessionBytesReceived { session_id: usize, bytes: usize },
    SessionPhaseStarted { session_id: usize, phase: String },
    SessionTurnCompleted { session_id: usize },
    SessionModelDetected { session_id: usize, name: String },
    SessionToolResultReceived { session_id: usize, tool_id: Option<String>, is_error: bool, agent_name: String },
    SessionStopReason { session_id: usize, reason: String },
    SessionWorkflowComplete { session_id: usize },
    SessionWorkflowError { session_id: usize, error: String },
    /// Workflow was cleanly stopped and a snapshot was saved
    SessionWorkflowStopped { session_id: usize },
    SessionGeneratingSummary { session_id: usize },

    SessionPlanGenerationFailed { session_id: usize, error: String },

    /// All reviewers failed after retry exhaustion - prompts for recovery decision
    SessionAllReviewersFailed { session_id: usize, summary: String },

    SessionMaxIterationsReached { session_id: usize, summary: String },

    SessionUserOverrideApproval { session_id: usize, summary: String },

    SessionAgentMessage {
        session_id: usize,
        agent_name: String,
        phase: String,
        message: String,
    },

    /// Generic workflow failure that can be recovered via retry/stop/abort
    SessionWorkflowFailure { session_id: usize, summary: String },

    SessionTodosUpdate {
        session_id: usize,
        agent_name: String,  
        todos: Vec<TodoItem>,
    },

    SessionRunTabSummaryGenerating {
        session_id: usize,
        phase: String,
        run_id: u64,
    },

    SessionRunTabSummaryReady {
        session_id: usize,
        phase: String,
        summary: String,
        run_id: u64,
    },

    SessionRunTabSummaryError {
        session_id: usize,
        phase: String,
        error: String,
        run_id: u64,
    },

    AccountUsageUpdate(AccountUsage),

    UpdateStatusReceived(UpdateStatus),

    VersionInfoReceived(Option<VersionInfo>),

    UpdateInstallFinished(UpdateResult),

    /// Output data from the embedded implementation terminal
    ImplementationOutput { session_id: usize, chunk: Vec<u8> },
    /// Implementation terminal process has exited
    ImplementationExited { session_id: usize, exit_code: Option<i32> },
    /// Implementation terminal encountered an error
    ImplementationError { session_id: usize, error: String },

    // Verification workflow events
    /// Verification phase started
    SessionVerificationStarted { session_id: usize, iteration: u32 },
    /// Verification phase completed with verdict
    SessionVerificationCompleted { session_id: usize, verdict: String, report: String },
    /// Fixing phase started
    SessionFixingStarted { session_id: usize, iteration: u32 },
    /// Fixing phase completed
    SessionFixingCompleted { session_id: usize },
    /// Verification workflow result
    SessionVerificationResult { session_id: usize, approved: bool, iterations_used: u32 },

    /// File index ready for @-mention auto-complete
    FileIndexReady(FileIndex),

    /// Slash command execution result
    SlashCommandResult {
        session_id: usize,
        command: String,
        summary: String,
        error: Option<String>,
    },

    /// Session browser async refresh completed
    SessionBrowserRefreshComplete {
        entries: Vec<crate::tui::session_browser::SessionEntry>,
        daemon_connected: bool,
        error: Option<String>,
    },

    /// Push notification from daemon: session state changed
    DaemonSessionChanged(crate::session_daemon::SessionRecord),

    /// Daemon subscription disconnected
    DaemonDisconnected,

    /// Daemon subscription reconnected
    DaemonReconnected,

    /// Request to save snapshots for all active sessions.
    /// Used by periodic auto-save and signal handlers.
    SnapshotRequest,
}

#[derive(Debug, Clone)]
pub enum UserApprovalResponse {
    Accept,
    Decline(String),
    ReviewRetry,
    ReviewContinue,

    PlanGenerationRetry,
    PlanGenerationContinue,  // Continue with existing plan file if available
    AbortWorkflow,
    ProceedWithoutApproval,
    ContinueReviewing,

    // Workflow failure recovery responses
    WorkflowFailureRetry,
    WorkflowFailureStop,
    WorkflowFailureAbort,
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        tokio::spawn(async move {
            let mut event_stream = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    maybe_event = event_stream.next() => {
                        match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                if key.kind == KeyEventKind::Press
                                    && event_tx.send(Event::Key(key)).is_err()
                                {
                                    break;
                                }
                            }
                            Some(Ok(CrosstermEvent::Paste(text))) => {
                                if event_tx.send(Event::Paste(text)).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(CrosstermEvent::Resize(_, _))) => {
                                if event_tx.send(Event::Resize).is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                    _ = tick_interval.tick() => {
                        if event_tx.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, _tx: tx }
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Event> {
        self._tx.clone()
    }

    pub async fn next(&mut self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
    }

    pub fn try_next(&mut self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

#[derive(Clone)]
pub struct SessionEventSender {
    session_id: usize,
    run_id: u64,
    inner: mpsc::UnboundedSender<Event>,
    /// Optional session logger for workflow events.
    session_logger: Option<Arc<SessionLogger>>,
}

/// Some methods may not be used in all code paths but are part of the
/// public API for workflow integration.
#[allow(dead_code)]
impl SessionEventSender {
    pub fn new(session_id: usize, run_id: u64, sender: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            session_id,
            run_id,
            inner: sender,
            session_logger: None,
        }
    }

    /// Creates a new SessionEventSender with a session logger attached.
    pub fn with_logger(
        session_id: usize,
        run_id: u64,
        sender: mpsc::UnboundedSender<Event>,
        logger: Arc<SessionLogger>,
    ) -> Self {
        Self {
            session_id,
            run_id,
            inner: sender,
            session_logger: Some(logger),
        }
    }

    /// Attaches a session logger to this sender.
    pub fn set_logger(&mut self, logger: Arc<SessionLogger>) {
        self.session_logger = Some(logger);
    }

    /// Returns the session logger if one is attached.
    pub fn logger(&self) -> Option<&Arc<SessionLogger>> {
        self.session_logger.as_ref()
    }

    pub fn session_id(&self) -> usize {
        self.session_id
    }

    /// Logs a message to the session logger if available.
    pub fn log(&self, level: LogLevel, category: LogCategory, message: &str) {
        if let Some(ref logger) = self.session_logger {
            logger.log(level, category, message);
        }
    }

    /// Logs an info-level workflow message.
    pub fn log_workflow(&self, message: &str) {
        self.log(LogLevel::Info, LogCategory::Workflow, message);
    }

    /// Logs a debug-level workflow message.
    pub fn log_workflow_debug(&self, message: &str) {
        self.log(LogLevel::Debug, LogCategory::Workflow, message);
    }

    pub fn send_output(&self, line: String) {
        let _ = self.inner.send(Event::SessionOutput {
            session_id: self.session_id,
            line,
        });
    }

    pub fn send_streaming(&self, line: String) {
        let _ = self.inner.send(Event::SessionStreaming {
            session_id: self.session_id,
            line,
        });
    }

    pub fn send_state_update(&self, state: State) {
        let _ = self.inner.send(Event::SessionStateUpdate {
            session_id: self.session_id,
            state,
        });
    }

    pub fn send_approval_request(&self, summary: String) {
        let _ = self.inner.send(Event::SessionApprovalRequest {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_review_decision_request(&self, summary: String) {
        let _ = self.inner.send(Event::SessionReviewDecisionRequest {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_token_usage(&self, usage: TokenUsage) {
        let _ = self.inner.send(Event::SessionTokenUsage {
            session_id: self.session_id,
            usage,
        });
    }

    pub fn send_tool_started(
        &self,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
    ) {
        let _ = self.inner.send(Event::SessionToolStarted {
            session_id: self.session_id,
            tool_id,
            display_name,
            input_preview,
            agent_name,
        });
    }

    pub fn send_tool_finished(&self, tool_id: Option<String>, agent_name: String) {
        let _ = self.inner.send(Event::SessionToolFinished {
            session_id: self.session_id,
            tool_id,
            agent_name,
        });
    }

    pub fn send_bytes_received(&self, bytes: usize) {
        let _ = self.inner.send(Event::SessionBytesReceived {
            session_id: self.session_id,
            bytes,
        });
    }

    pub fn send_phase_started(&self, phase: String) {
        let _ = self.inner.send(Event::SessionPhaseStarted {
            session_id: self.session_id,
            phase,
        });
    }

    pub fn send_turn_completed(&self) {
        let _ = self.inner.send(Event::SessionTurnCompleted {
            session_id: self.session_id,
        });
    }

    pub fn send_model_detected(&self, name: String) {
        let _ = self.inner.send(Event::SessionModelDetected {
            session_id: self.session_id,
            name,
        });
    }

    pub fn send_tool_result_received(&self, tool_id: Option<String>, is_error: bool, agent_name: String) {
        let _ = self.inner.send(Event::SessionToolResultReceived {
            session_id: self.session_id,
            tool_id,
            is_error,
            agent_name,
        });
    }

    pub fn send_stop_reason(&self, reason: String) {
        let _ = self.inner.send(Event::SessionStopReason {
            session_id: self.session_id,
            reason,
        });
    }

    pub fn send_workflow_complete(&self) {
        let _ = self.inner.send(Event::SessionWorkflowComplete {
            session_id: self.session_id,
        });
    }

    pub fn send_workflow_error(&self, error: String) {
        let _ = self.inner.send(Event::SessionWorkflowError {
            session_id: self.session_id,
            error,
        });
    }

    pub fn send_workflow_stopped(&self) {
        let _ = self.inner.send(Event::SessionWorkflowStopped {
            session_id: self.session_id,
        });
    }

    pub fn send_generating_summary(&self) {
        let _ = self.inner.send(Event::SessionGeneratingSummary {
            session_id: self.session_id,
        });
    }

    pub fn send_plan_generation_failed(&self, error: String) {
        let _ = self.inner.send(Event::SessionPlanGenerationFailed {
            session_id: self.session_id,
            error,
        });
    }

    pub fn send_all_reviewers_failed(&self, summary: String) {
        let _ = self.inner.send(Event::SessionAllReviewersFailed {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_workflow_failure(&self, summary: String) {
        let _ = self.inner.send(Event::SessionWorkflowFailure {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_max_iterations_reached(&self, summary: String) {
        let _ = self.inner.send(Event::SessionMaxIterationsReached {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_user_override_approval(&self, summary: String) {
        let _ = self.inner.send(Event::SessionUserOverrideApproval {
            session_id: self.session_id,
            summary,
        });
    }

    pub fn send_agent_message(&self, agent_name: String, phase: String, message: String) {
        let _ = self.inner.send(Event::SessionAgentMessage {
            session_id: self.session_id,
            agent_name,
            phase,
            message,
        });
    }

    pub fn send_todos_update(&self, agent_name: String, todos: Vec<TodoItem>) {
        let _ = self.inner.send(Event::SessionTodosUpdate {
            session_id: self.session_id,
            agent_name,
            todos,
        });
    }

    pub fn send_run_tab_summary_generating(&self, phase: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryGenerating {
            session_id: self.session_id,
            phase,
            run_id: self.run_id,
        });
    }

    pub fn send_run_tab_summary_ready(&self, phase: String, summary: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryReady {
            session_id: self.session_id,
            phase,
            summary,
            run_id: self.run_id,
        });
    }

    pub fn send_run_tab_summary_error(&self, phase: String, error: String) {
        let _ = self.inner.send(Event::SessionRunTabSummaryError {
            session_id: self.session_id,
            phase,
            error,
            run_id: self.run_id,
        });
    }

    pub fn raw_sender(&self) -> mpsc::UnboundedSender<Event> {
        self.inner.clone()
    }

    // Verification workflow event helpers

    pub fn send_verification_started(&self, iteration: u32) {
        let _ = self.inner.send(Event::SessionVerificationStarted {
            session_id: self.session_id,
            iteration,
        });
    }

    pub fn send_verification_completed(&self, verdict: String, report: String) {
        let _ = self.inner.send(Event::SessionVerificationCompleted {
            session_id: self.session_id,
            verdict,
            report,
        });
    }

    pub fn send_fixing_started(&self, iteration: u32) {
        let _ = self.inner.send(Event::SessionFixingStarted {
            session_id: self.session_id,
            iteration,
        });
    }

    pub fn send_fixing_completed(&self) {
        let _ = self.inner.send(Event::SessionFixingCompleted {
            session_id: self.session_id,
        });
    }

    pub fn send_verification_result(&self, approved: bool, iterations_used: u32) {
        let _ = self.inner.send(Event::SessionVerificationResult {
            session_id: self.session_id,
            approved,
            iterations_used,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_includes_session_id() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = SessionEventSender::new(42, 0, tx);

        sender.send_output("test line".to_string());

        let event = rx.try_recv().unwrap();
        match event {
            Event::SessionOutput { session_id, line } => {
                assert_eq!(session_id, 42);
                assert_eq!(line, "test line");
            }
            _ => panic!("Expected SessionOutput event"),
        }
    }

    #[test]
    fn test_multiple_senders() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let sender1 = SessionEventSender::new(1, 0, tx.clone());
        let sender2 = SessionEventSender::new(2, 0, tx);

        sender1.send_output("from session 1".to_string());
        sender2.send_output("from session 2".to_string());

        let event1 = rx.try_recv().unwrap();
        let event2 = rx.try_recv().unwrap();

        match event1 {
            Event::SessionOutput { session_id, .. } => assert_eq!(session_id, 1),
            _ => panic!("Expected SessionOutput event"),
        }

        match event2 {
            Event::SessionOutput { session_id, .. } => assert_eq!(session_id, 2),
            _ => panic!("Expected SessionOutput event"),
        }
    }

    #[test]
    fn test_summary_events_include_run_id() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = SessionEventSender::new(1, 42, tx);

        sender.send_run_tab_summary_generating("Planning".to_string());
        sender.send_run_tab_summary_ready("Planning".to_string(), "Summary content".to_string());
        sender.send_run_tab_summary_error("Planning".to_string(), "Error message".to_string());

        match rx.try_recv().unwrap() {
            Event::SessionRunTabSummaryGenerating { session_id, phase, run_id } => {
                assert_eq!(session_id, 1);
                assert_eq!(phase, "Planning");
                assert_eq!(run_id, 42);
            }
            _ => panic!("Expected SessionRunTabSummaryGenerating event"),
        }

        match rx.try_recv().unwrap() {
            Event::SessionRunTabSummaryReady { session_id, phase, summary, run_id } => {
                assert_eq!(session_id, 1);
                assert_eq!(phase, "Planning");
                assert_eq!(summary, "Summary content");
                assert_eq!(run_id, 42);
            }
            _ => panic!("Expected SessionRunTabSummaryReady event"),
        }

        match rx.try_recv().unwrap() {
            Event::SessionRunTabSummaryError { session_id, phase, error, run_id } => {
                assert_eq!(session_id, 1);
                assert_eq!(phase, "Planning");
                assert_eq!(error, "Error message");
                assert_eq!(run_id, 42);
            }
            _ => panic!("Expected SessionRunTabSummaryError event"),
        }
    }
}
