//! Workflow events for the CQRS aggregate.
//!
//! Events represent facts that have happened. They are the single source of truth
//! for the workflow state and are persisted to the event log.

use crate::domain::failure::FailureContext;
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentId, ConversationId, FeatureName, FeedbackPath, ImplementationVerdict, Iteration,
    MaxIterations, Objective, PhaseLabel, PlanPath, ResumeStrategy, TimestampUtc, WorkingDir,
    WorktreeState,
};
use cqrs_es::DomainEvent;
use serde::{Deserialize, Serialize};

/// Events emitted by the workflow aggregate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEvent {
    /// Workflow was created.
    WorkflowCreated {
        feature_name: FeatureName,
        objective: Objective,
        working_dir: WorkingDir,
        max_iterations: MaxIterations,
        plan_path: PlanPath,
        feedback_path: FeedbackPath,
        created_at: TimestampUtc,
    },

    /// Planning phase started.
    PlanningStarted { started_at: TimestampUtc },

    /// Planning completed with a plan file.
    PlanningCompleted {
        plan_path: PlanPath,
        completed_at: TimestampUtc,
    },

    /// Review cycle started.
    ReviewCycleStarted {
        mode: ReviewMode,
        reviewers: Vec<AgentId>,
        started_at: TimestampUtc,
    },

    /// Reviewer approved the plan.
    ReviewerApproved {
        reviewer_id: AgentId,
        approved_at: TimestampUtc,
    },

    /// Reviewer rejected the plan.
    ReviewerRejected {
        reviewer_id: AgentId,
        feedback_path: FeedbackPath,
        rejected_at: TimestampUtc,
    },

    /// Review cycle completed.
    ReviewCycleCompleted {
        approved: bool,
        completed_at: TimestampUtc,
    },

    /// Revising phase started.
    RevisingStarted {
        feedback_summary: String,
        started_at: TimestampUtc,
    },

    /// Revision completed with updated plan.
    RevisionCompleted {
        plan_path: PlanPath,
        completed_at: TimestampUtc,
    },

    /// Max iterations reached during planning.
    PlanningMaxIterationsReached { reached_at: TimestampUtc },

    /// User approved the plan.
    UserApproved { approved_at: TimestampUtc },

    /// User requested implementation.
    UserRequestedImplementation { requested_at: TimestampUtc },

    /// User declined with feedback.
    UserDeclined {
        feedback: String,
        declined_at: TimestampUtc,
    },

    /// User aborted the workflow.
    UserAborted {
        reason: String,
        aborted_at: TimestampUtc,
    },

    /// User overrode approval.
    UserOverrideApproval {
        override_reason: String,
        overridden_at: TimestampUtc,
    },

    /// Implementation started.
    ImplementationStarted {
        max_iterations: MaxIterations,
        started_at: TimestampUtc,
    },

    /// Implementation round started.
    ImplementationRoundStarted {
        iteration: Iteration,
        started_at: TimestampUtc,
    },

    /// Implementation round completed.
    ImplementationRoundCompleted {
        iteration: Iteration,
        fingerprint: u64,
        completed_at: TimestampUtc,
    },

    /// Implementation review completed.
    ImplementationReviewCompleted {
        iteration: Iteration,
        verdict: ImplementationVerdict,
        feedback: Option<String>,
        completed_at: TimestampUtc,
    },

    /// Implementation max iterations reached.
    ImplementationMaxIterationsReached { reached_at: TimestampUtc },

    /// Implementation accepted.
    ImplementationAccepted { approved_at: TimestampUtc },

    /// Implementation declined.
    ImplementationDeclined {
        reason: String,
        declined_at: TimestampUtc,
    },

    /// Implementation cancelled.
    ImplementationCancelled {
        reason: String,
        cancelled_at: TimestampUtc,
    },

    /// Agent conversation recorded.
    AgentConversationRecorded {
        agent_id: AgentId,
        resume_strategy: ResumeStrategy,
        conversation_id: Option<ConversationId>,
        updated_at: TimestampUtc,
    },

    /// Invocation recorded.
    InvocationRecorded {
        agent_id: AgentId,
        phase: PhaseLabel,
        timestamp: TimestampUtc,
        conversation_id: Option<ConversationId>,
        resume_strategy: ResumeStrategy,
    },

    /// Failure recorded.
    FailureRecorded {
        failure: FailureContext,
        recorded_at: TimestampUtc,
    },

    /// Worktree attached.
    WorktreeAttached { worktree_state: WorktreeState },
}

impl DomainEvent for WorkflowEvent {
    fn event_type(&self) -> String {
        match self {
            Self::WorkflowCreated { .. } => "WorkflowCreated".to_string(),
            Self::PlanningStarted { .. } => "PlanningStarted".to_string(),
            Self::PlanningCompleted { .. } => "PlanningCompleted".to_string(),
            Self::ReviewCycleStarted { .. } => "ReviewCycleStarted".to_string(),
            Self::ReviewerApproved { .. } => "ReviewerApproved".to_string(),
            Self::ReviewerRejected { .. } => "ReviewerRejected".to_string(),
            Self::ReviewCycleCompleted { .. } => "ReviewCycleCompleted".to_string(),
            Self::RevisingStarted { .. } => "RevisingStarted".to_string(),
            Self::RevisionCompleted { .. } => "RevisionCompleted".to_string(),
            Self::PlanningMaxIterationsReached { .. } => "PlanningMaxIterationsReached".to_string(),
            Self::UserApproved { .. } => "UserApproved".to_string(),
            Self::UserRequestedImplementation { .. } => "UserRequestedImplementation".to_string(),
            Self::UserDeclined { .. } => "UserDeclined".to_string(),
            Self::UserAborted { .. } => "UserAborted".to_string(),
            Self::UserOverrideApproval { .. } => "UserOverrideApproval".to_string(),
            Self::ImplementationStarted { .. } => "ImplementationStarted".to_string(),
            Self::ImplementationRoundStarted { .. } => "ImplementationRoundStarted".to_string(),
            Self::ImplementationRoundCompleted { .. } => "ImplementationRoundCompleted".to_string(),
            Self::ImplementationReviewCompleted { .. } => {
                "ImplementationReviewCompleted".to_string()
            }
            Self::ImplementationMaxIterationsReached { .. } => {
                "ImplementationMaxIterationsReached".to_string()
            }
            Self::ImplementationAccepted { .. } => "ImplementationAccepted".to_string(),
            Self::ImplementationDeclined { .. } => "ImplementationDeclined".to_string(),
            Self::ImplementationCancelled { .. } => "ImplementationCancelled".to_string(),
            Self::AgentConversationRecorded { .. } => "AgentConversationRecorded".to_string(),
            Self::InvocationRecorded { .. } => "InvocationRecorded".to_string(),
            Self::FailureRecorded { .. } => "FailureRecorded".to_string(),
            Self::WorktreeAttached { .. } => "WorktreeAttached".to_string(),
        }
    }

    fn event_version(&self) -> String {
        "1".to_string()
    }
}
