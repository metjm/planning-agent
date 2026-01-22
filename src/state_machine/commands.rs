//! Commands that can mutate workflow state.
//!
//! All state changes MUST go through the state machine's `apply()` method.
//! This is the only way to mutate state, ensuring a single source of truth.

use std::path::PathBuf;

/// Commands that can mutate workflow state.
/// All state changes MUST go through the state machine's apply() method.
#[derive(Debug, Clone)]
pub enum StateCommand {
    // Phase transitions
    /// Start planning phase (validates current state is Planning)
    StartPlanning,
    /// Complete planning with the plan file path, transitions to Reviewing
    CompletePlanning { plan_path: PathBuf },
    /// Start reviewing with a specific reviewer
    StartReviewing { reviewer_id: String },
    /// A reviewer approved the plan
    ReviewerApproved { reviewer_id: String },
    /// A reviewer rejected the plan with feedback
    ReviewerRejected {
        reviewer_id: String,
        feedback_path: PathBuf,
    },
    /// All reviewers have completed their reviews
    AllReviewersComplete { approved: bool },
    /// Start revising phase with feedback content
    StartRevising { feedback_content: String },
    /// Complete revising, increment iteration, transition to Reviewing
    CompleteRevising,
    /// Mark workflow as complete
    MarkComplete,

    // User actions
    /// User approved the plan
    UserApprove,
    /// User requested implementation
    UserRequestImplementation,
    /// User declined with feedback
    UserDecline { feedback: String },
    /// User aborted the workflow
    UserAbort { reason: String },
    /// User overrode approval (bypasses normal validation)
    UserOverrideApproval,

    // Iteration management
    /// Increment the iteration counter
    IncrementIteration,
    /// Extend max iterations by 1
    ExtendMaxIterations,

    // Agent tracking
    /// Update agent conversation ID for resume
    UpdateAgentConversation {
        agent: String,
        conversation_id: String,
    },
    /// Record an agent invocation
    RecordInvocation { agent: String, phase: String },

    // Error handling
    /// Record an agent failure
    AgentFailed { agent_id: String, error: String },
    /// Clear the current failure state
    ClearFailure,

    // Sequential review management
    /// Initialize sequential review state (creates new SequentialReviewState::new())
    InitSequentialReview,
    /// Clear sequential review state
    ClearSequentialReview,
    /// Advance to the next reviewer in sequential mode
    AdvanceSequentialReviewer,

    // Restart workflow with user feedback (resets to Planning phase)
    /// Restart workflow with user feedback, resets to Planning phase
    RestartWithFeedback { feedback: String },
}
