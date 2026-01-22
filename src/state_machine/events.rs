//! Events emitted by the state machine after processing commands.
//!
//! These are for logging and notification purposes only - not for TUI state updates.
//! TUI gets updates via the watch channel's StateSnapshot.

use crate::state::Phase;
use serde::Serialize;

/// Events emitted by the state machine after processing commands.
/// These are for logging and notification purposes only - not for TUI state updates.
/// TUI gets updates via the watch channel's StateSnapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum StateEvent {
    /// Phase changed from one phase to another
    PhaseChanged { from: Phase, to: Phase },
    /// Iteration was incremented
    IterationIncremented { new_value: u32 },
    /// Iteration was reset (e.g., on restart)
    IterationReset,
    /// A reviewer's status changed (approved or rejected)
    ReviewerStatusChanged { reviewer_id: String, approved: bool },
    /// Workflow completed successfully
    WorkflowComplete { approved: bool, override_used: bool },
    /// Workflow was restarted with user feedback
    WorkflowRestarted { feedback_preview: String },
    /// An error occurred
    ErrorOccurred { error: String },
    /// Error was cleared
    ErrorCleared,
    /// Agent conversation ID was updated
    AgentConversationUpdated { agent: String },
    /// An agent invocation was recorded
    InvocationRecorded { agent: String, phase: String },
}
