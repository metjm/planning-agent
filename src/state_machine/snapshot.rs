//! Read-only snapshot of workflow state for TUI display.
//!
//! TUI NEVER mutates this; it receives new snapshots via watch channel.

use crate::state::{
    AgentConversationState, ImplementationPhaseState, Phase, SequentialReviewState, UiMode,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Read-only snapshot of state for TUI display.
/// Contains ALL fields needed for UI rendering (verified via codebase search).
/// TUI NEVER mutates this; it receives new snapshots via watch channel.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    // Core workflow state
    /// Current workflow phase
    pub phase: Phase,
    /// Current iteration number (1-indexed)
    pub iteration: u32,
    /// Maximum allowed iterations
    pub max_iterations: u32,
    /// Feature name for display
    pub feature_name: String,
    /// Workflow session ID (UUID)
    pub workflow_session_id: String,

    // Fields accessed by TUI (verified via codebase search)
    /// Planning objective (panels.rs:40-44, objective.rs:52)
    pub objective: String,
    /// Plan file path (overlays.rs:375, mod.rs:118)
    pub plan_file: PathBuf,
    /// Implementation phase state (overlays.rs:20, session/mod.rs:581)
    pub implementation_state: Option<ImplementationPhaseState>,
    /// Agent conversation states for resume (session/mod.rs:597-600)
    pub agent_conversations: HashMap<String, AgentConversationState>,

    // Additional useful fields
    /// Whether approval was overridden by user
    pub approval_overridden: bool,
    /// Current UI mode (Planning or Implementation)
    pub ui_mode: UiMode,
    /// Sequential review tracking state
    pub sequential_review: Option<SequentialReviewState>,
    /// Whether there's an active failure
    pub has_failure: bool,
}

impl From<&crate::state::State> for StateSnapshot {
    fn from(state: &crate::state::State) -> Self {
        Self {
            phase: state.phase.clone(),
            iteration: state.iteration,
            max_iterations: state.max_iterations,
            feature_name: state.feature_name.clone(),
            workflow_session_id: state.workflow_session_id.clone(),
            objective: state.objective.clone(),
            plan_file: state.plan_file.clone(),
            implementation_state: state.implementation_state.clone(),
            agent_conversations: state.agent_conversations.clone(),
            approval_overridden: state.approval_overridden,
            ui_mode: state.workflow_stage(),
            sequential_review: state.sequential_review.clone(),
            has_failure: state.last_failure.is_some(),
        }
    }
}
