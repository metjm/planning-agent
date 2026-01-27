//! Workflow commands for the CQRS aggregate.
//!
//! Commands represent intent to change state. The aggregate validates commands
//! and produces events that are persisted to the event log.

use crate::domain::failure::FailureContext;
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentId, ConversationId, FeatureName, FeedbackPath, ImplementationVerdict, Iteration,
    MaxIterations, Objective, PhaseLabel, PlanPath, ResumeStrategy, WorkingDir, WorktreeState,
};
use serde::{Deserialize, Serialize};

/// Commands that can be executed against the workflow aggregate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCommand {
    /// Initialize aggregate state for a new workflow.
    CreateWorkflow {
        feature_name: FeatureName,
        objective: Objective,
        working_dir: WorkingDir,
        max_iterations: MaxIterations,
        plan_path: PlanPath,
        feedback_path: FeedbackPath,
    },

    /// Begin planning phase (idempotent when already planning).
    StartPlanning,

    /// Planning agent produced a plan file.
    PlanningCompleted { plan_path: PlanPath },

    /// Begin review cycle for current iteration.
    ReviewCycleStarted {
        mode: ReviewMode,
        reviewers: Vec<AgentId>,
    },

    /// Record reviewer approval.
    ReviewerApproved { reviewer_id: AgentId },

    /// Record reviewer rejection.
    ReviewerRejected {
        reviewer_id: AgentId,
        feedback_path: FeedbackPath,
    },

    /// Aggregate review result to move to Revising or Complete.
    ReviewCycleCompleted { approved: bool },

    /// Begin revision work.
    /// When dispatched from AwaitingPlanningDecision, additional_iterations specifies
    /// how many more iterations to allow (default 1 if not specified).
    RevisingStarted {
        feedback_summary: String,
        /// Only set when continuing from AwaitingPlanningDecision.
        /// Triggers MaxIterationsExtended event before RevisingStarted.
        additional_iterations: Option<u32>,
    },

    /// Revision finished and new plan saved.
    RevisionCompleted { plan_path: PlanPath },

    /// Enter AwaitingDecision phase due to max iterations.
    PlanningMaxIterationsReached,

    /// User approved plan.
    UserApproved,

    /// User requested implementation workflow.
    /// This command emits both UserRequestedImplementation and ImplementationStarted events.
    UserRequestedImplementation,

    /// User declined with feedback.
    UserDeclined { feedback: String },

    /// User aborted workflow.
    UserAborted { reason: String },

    /// User bypassed review (approval_overridden=true).
    UserOverrideApproval { override_reason: String },

    /// Internal: emitted alongside UserRequestedImplementation to start implementation.
    /// Direct commands of this type are rejected.
    ImplementationStarted { max_iterations: MaxIterations },

    /// Implementation round started.
    ImplementationRoundStarted { iteration: Iteration },

    /// Implementation round completed.
    ImplementationRoundCompleted {
        iteration: Iteration,
        fingerprint: u64,
    },

    /// Implementation review completed.
    ImplementationReviewCompleted {
        iteration: Iteration,
        verdict: ImplementationVerdict,
        feedback: Option<String>,
    },

    /// Awaiting max-iterations decision for implementation.
    ImplementationMaxIterationsReached,

    /// Implementation accepted.
    ImplementationAccepted,

    /// Implementation declined.
    ImplementationDeclined { reason: String },

    /// Implementation cancelled by user.
    ImplementationCancelled { reason: String },

    /// Persist agent conversation state.
    RecordAgentConversation {
        agent_id: AgentId,
        resume_strategy: ResumeStrategy,
        conversation_id: Option<ConversationId>,
    },

    /// Persist invocation record.
    RecordInvocation {
        agent_id: AgentId,
        phase: PhaseLabel,
        conversation_id: Option<ConversationId>,
        resume_strategy: ResumeStrategy,
    },

    /// Persist failure context.
    RecordFailure { failure: FailureContext },

    /// Persist worktree metadata.
    AttachWorktree { worktree_state: WorktreeState },
}
