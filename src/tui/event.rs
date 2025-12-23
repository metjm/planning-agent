use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent, KeyEventKind};
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::cli_usage::AccountUsage;
use crate::state::State;

/// Token usage statistics from Claude API
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Event {
    Key(KeyEvent),
    Paste(String),
    Tick,
    Resize,

    // Events without session routing (global)
    Output(String),
    Streaming(String),
    ToolStarted(String),
    ToolFinished(String),
    StateUpdate(State),
    RequestUserApproval(String),
    BytesReceived(usize),
    TokenUsage(TokenUsage),
    PhaseStarted(String),
    TurnCompleted,
    ModelDetected(String),
    ToolResultReceived { tool_id: String, is_error: bool },
    StopReason(String),

    // Session-routed events (for multi-tab support)
    SessionOutput { session_id: usize, line: String },
    SessionStreaming { session_id: usize, line: String },
    SessionStateUpdate { session_id: usize, state: State },
    SessionApprovalRequest { session_id: usize, summary: String },
    SessionReviewDecisionRequest { session_id: usize, summary: String },
    SessionTokenUsage { session_id: usize, usage: TokenUsage },
    SessionToolStarted { session_id: usize, name: String },
    SessionToolFinished { session_id: usize, id: String },
    SessionBytesReceived { session_id: usize, bytes: usize },
    SessionPhaseStarted { session_id: usize, phase: String },
    SessionTurnCompleted { session_id: usize },
    SessionModelDetected { session_id: usize, name: String },
    SessionToolResultReceived { session_id: usize, tool_id: String, is_error: bool },
    SessionStopReason { session_id: usize, reason: String },
    SessionWorkflowComplete { session_id: usize },
    SessionWorkflowError { session_id: usize, error: String },
    SessionGeneratingSummary { session_id: usize },

    /// Plan generation failed, user can retry
    SessionPlanGenerationFailed { session_id: usize, error: String },
    /// Max iterations reached, user can proceed or continue
    SessionMaxIterationsReached { session_id: usize, summary: String },
    /// User chose to proceed without AI approval, now awaiting final confirmation
    SessionUserOverrideApproval { session_id: usize, summary: String },

    /// Agent chat message for display in chat tabs (session-routed)
    SessionAgentMessage {
        session_id: usize,
        agent_name: String,
        phase: String,   // "Planning", "Reviewing #1", "Revising #1", etc.
        message: String,
    },

    /// Account usage update for all providers (global, not per-session)
    AccountUsageUpdate(AccountUsage),
}

/// User's response to approval request
#[derive(Debug, Clone)]
pub enum UserApprovalResponse {
    Accept,
    Decline(String), // Contains user's feedback for changes
    ReviewRetry,
    ReviewContinue,
    // Failure recovery variants
    PlanGenerationRetry,    // Retry plan generation after failure
    AbortWorkflow,          // Explicit abort (ends session, not app)
    ProceedWithoutApproval, // User wants to proceed despite max iterations
    ContinueReviewing,      // User wants another review cycle (adds to max_iterations)
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        // Spawn keyboard/tick event handler
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

    /// Try to receive an event without blocking
    /// Returns None if no event is available
    pub fn try_next(&mut self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

/// A sender that automatically tags events with a session ID
/// This is passed to workflow tasks to ensure events are routed correctly
#[derive(Clone)]
#[allow(dead_code)]
pub struct SessionEventSender {
    session_id: usize,
    inner: mpsc::UnboundedSender<Event>,
}

#[allow(dead_code)]
impl SessionEventSender {
    pub fn new(session_id: usize, sender: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            session_id,
            inner: sender,
        }
    }

    pub fn session_id(&self) -> usize {
        self.session_id
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

    pub fn send_tool_started(&self, name: String) {
        let _ = self.inner.send(Event::SessionToolStarted {
            session_id: self.session_id,
            name,
        });
    }

    pub fn send_tool_finished(&self, id: String) {
        let _ = self.inner.send(Event::SessionToolFinished {
            session_id: self.session_id,
            id,
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

    pub fn send_tool_result_received(&self, tool_id: String, is_error: bool) {
        let _ = self.inner.send(Event::SessionToolResultReceived {
            session_id: self.session_id,
            tool_id,
            is_error,
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

    /// Send an agent chat message for display in the chat tabs
    pub fn send_agent_message(&self, agent_name: String, phase: String, message: String) {
        let _ = self.inner.send(Event::SessionAgentMessage {
            session_id: self.session_id,
            agent_name,
            phase,
            message,
        });
    }

    /// Get the raw sender for compatibility with code that expects UnboundedSender<Event>
    pub fn raw_sender(&self) -> mpsc::UnboundedSender<Event> {
        self.inner.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_includes_session_id() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sender = SessionEventSender::new(42, tx);

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

        let sender1 = SessionEventSender::new(1, tx.clone());
        let sender2 = SessionEventSender::new(2, tx);

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
}
