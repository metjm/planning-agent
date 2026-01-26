use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, KeyEventKind, MouseEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::app::workflow_decisions::IterativePhase;
use crate::cli_usage::AccountUsage;
use crate::domain::view::WorkflowView;
use crate::state::State;
use crate::tui::file_index::FileIndex;
use crate::tui::session::{CliInstanceId, ReviewKind, TodoItem, ToolResultSummary};
use crate::update::{UpdateResult, UpdateStatus, VersionInfo};
use std::time::Instant;

// Re-export SessionEventSender from its module
pub use super::session_event_sender::SessionEventSender;

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
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Tick,
    Resize,

    Output(String),
    StateUpdate(State),

    SessionOutput {
        session_id: usize,
        line: String,
    },
    SessionStreaming {
        session_id: usize,
        line: String,
    },
    SessionStateUpdate {
        session_id: usize,
        state: State,
    },
    /// New event-sourced view update (replaces SessionStateUpdate in CQRS mode)
    SessionViewUpdate {
        session_id: usize,
        view: WorkflowView,
    },
    SessionApprovalRequest {
        session_id: usize,
        summary: String,
    },
    SessionReviewDecisionRequest {
        session_id: usize,
        summary: String,
    },

    /// A review round has started
    SessionReviewRoundStarted {
        session_id: usize,
        kind: ReviewKind,
        round: u32,
    },
    /// A reviewer has started within a round
    SessionReviewerStarted {
        session_id: usize,
        kind: ReviewKind,
        round: u32,
        display_id: String,
    },
    /// A reviewer has completed within a round
    SessionReviewerCompleted {
        session_id: usize,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        approved: bool,
        summary: String,
        duration_ms: u64,
    },
    /// A reviewer has failed within a round
    SessionReviewerFailed {
        session_id: usize,
        kind: ReviewKind,
        round: u32,
        display_id: String,
        error: String,
    },
    /// A review round has completed with aggregate verdict
    SessionReviewRoundCompleted {
        session_id: usize,
        kind: ReviewKind,
        round: u32,
        approved: bool,
    },

    SessionTokenUsage {
        session_id: usize,
        usage: TokenUsage,
    },
    SessionToolStarted {
        session_id: usize,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
        phase: String,
    },
    SessionToolFinished {
        session_id: usize,
        tool_id: Option<String>,
        agent_name: String,
    },
    SessionBytesReceived {
        session_id: usize,
        bytes: usize,
    },
    SessionPhaseStarted {
        session_id: usize,
        phase: String,
    },
    SessionTurnCompleted {
        session_id: usize,
    },
    SessionModelDetected {
        session_id: usize,
        name: String,
    },
    SessionToolResultReceived {
        session_id: usize,
        tool_id: Option<String>,
        is_error: bool,
        agent_name: String,
        phase: String,
        summary: ToolResultSummary,
    },
    SessionStopReason {
        session_id: usize,
        reason: String,
    },

    SessionPlanGenerationFailed {
        session_id: usize,
        error: String,
    },

    /// All reviewers failed after retry exhaustion - prompts for recovery decision
    SessionAllReviewersFailed {
        session_id: usize,
        summary: String,
    },

    /// Max iterations reached - prompt user for decision
    SessionMaxIterationsReached {
        session_id: usize,
        phase: IterativePhase,
        summary: String,
    },

    SessionUserOverrideApproval {
        session_id: usize,
        summary: String,
    },

    SessionAgentMessage {
        session_id: usize,
        agent_name: String,
        phase: String,
        message: String,
    },

    /// Generic workflow failure that can be recovered via retry/stop/abort
    SessionWorkflowFailure {
        session_id: usize,
        summary: String,
    },

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

    /// CLI instance lifecycle events
    SessionCliInstanceStarted {
        session_id: usize,
        id: CliInstanceId,
        agent_name: String,
        pid: Option<u32>,
        started_at: Instant,
    },
    SessionCliInstanceActivity {
        session_id: usize,
        id: CliInstanceId,
        activity_at: Instant,
    },
    SessionCliInstanceFinished {
        session_id: usize,
        id: CliInstanceId,
    },

    /// File index ready for @-mention auto-complete
    FileIndexReady(FileIndex),

    /// Slash command execution result
    SlashCommandResult {
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

    /// Implementation workflow was approved - show success modal
    SessionImplementationSuccess {
        session_id: usize,
        iterations_used: u32,
    },
    /// Implementation follow-up interaction finished (success or error)
    SessionImplementationInteractionFinished {
        session_id: usize,
    },
}

#[derive(Debug, Clone)]
pub enum UserApprovalResponse {
    Accept,
    /// Accept and start implementation workflow
    Implement,
    Decline(String),
    ReviewRetry,
    ReviewContinue,

    PlanGenerationRetry,
    PlanGenerationContinue, // Continue with existing plan file if available
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
                            Some(Ok(CrosstermEvent::Mouse(mouse))) => {
                                use crossterm::event::{MouseButton, MouseEventKind};
                                // Forward scroll and left-click events, ignore move/drag/right-click
                                if matches!(
                                    mouse.kind,
                                    MouseEventKind::ScrollUp
                                        | MouseEventKind::ScrollDown
                                        | MouseEventKind::Down(MouseButton::Left)
                                ) && event_tx.send(Event::Mouse(mouse)).is_err()
                                {
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
