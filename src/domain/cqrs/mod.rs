//! CQRS core types for event sourcing.
//!
//! This module contains the core CQRS types:
//! - **Commands**: Intent to change state
//! - **Events**: Facts that have happened
//! - **Aggregate**: Command validation and event application
//! - **Query**: Read-side queries

pub mod commands;
pub mod events;
pub mod query;

pub use commands::WorkflowCommand;
pub use events::WorkflowEvent;
pub use query::WorkflowQuery;

use crate::domain::errors::WorkflowError;
use crate::domain::failure::{FailureContext, MAX_FAILURE_HISTORY};
use crate::domain::review::ReviewMode;
use crate::domain::services::WorkflowServices;
use crate::domain::types::{
    AgentConversationState, AgentId, FeatureName, FeedbackPath, FeedbackStatus,
    ImplementationPhase, ImplementationPhaseState, InvocationRecord, Iteration, MaxIterations,
    Objective, Phase, PlanPath, TimestampUtc, WorkingDir, WorktreeState,
};
use async_trait::async_trait;
use cqrs_es::Aggregate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Active workflow data when the aggregate is initialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowData {
    pub feature_name: FeatureName,
    pub objective: Objective,
    pub working_dir: WorkingDir,
    pub created_at: TimestampUtc,
    pub planning_phase: Phase,
    pub iteration: Iteration,
    pub max_iterations: MaxIterations,
    pub plan_path: PlanPath,
    pub feedback_path: FeedbackPath,
    pub last_feedback_status: Option<FeedbackStatus>,
    pub review_mode: Option<ReviewMode>,
    pub approval_overridden: bool,
    pub implementation_state: Option<ImplementationPhaseState>,
    pub agent_conversations: HashMap<AgentId, AgentConversationState>,
    pub invocations: Vec<InvocationRecord>,
    pub last_failure: Option<FailureContext>,
    pub failure_history: Vec<FailureContext>,
    pub worktree_info: Option<WorktreeState>,
}

/// Workflow aggregate state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum WorkflowState {
    /// Aggregate has not been initialized.
    #[default]
    Uninitialized,
    /// Aggregate is active with workflow data (boxed for memory efficiency).
    Active(Box<WorkflowData>),
}

/// The workflow aggregate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowAggregate {
    pub state: WorkflowState,
}

#[async_trait]
impl Aggregate for WorkflowAggregate {
    type Command = WorkflowCommand;
    type Event = WorkflowEvent;
    type Error = WorkflowError;
    type Services = WorkflowServices;

    fn aggregate_type() -> String {
        "workflow".to_string()
    }

    async fn handle(
        &self,
        command: Self::Command,
        services: &Self::Services,
    ) -> Result<Vec<Self::Event>, Self::Error> {
        let now = services.clock.now();

        match (&self.state, command) {
            // CreateWorkflow - only valid on uninitialized aggregate
            (
                WorkflowState::Uninitialized,
                WorkflowCommand::CreateWorkflow {
                    feature_name,
                    objective,
                    working_dir,
                    max_iterations,
                    plan_path,
                    feedback_path,
                },
            ) => Ok(vec![WorkflowEvent::WorkflowCreated {
                feature_name,
                objective,
                working_dir,
                max_iterations,
                plan_path,
                feedback_path,
                created_at: now,
            }]),

            // StartPlanning - idempotent when already planning
            (WorkflowState::Active(data), WorkflowCommand::StartPlanning)
                if data.planning_phase == Phase::Planning =>
            {
                Ok(vec![WorkflowEvent::PlanningStarted { started_at: now }])
            }

            // PlanningCompleted
            (WorkflowState::Active(data), WorkflowCommand::PlanningCompleted { plan_path })
                if data.planning_phase == Phase::Planning =>
            {
                Ok(vec![WorkflowEvent::PlanningCompleted {
                    plan_path,
                    completed_at: now,
                }])
            }

            // ReviewCycleStarted
            (
                WorkflowState::Active(data),
                WorkflowCommand::ReviewCycleStarted { mode, reviewers },
            ) if data.planning_phase == Phase::Reviewing => {
                Ok(vec![WorkflowEvent::ReviewCycleStarted {
                    mode,
                    reviewers,
                    started_at: now,
                }])
            }

            // ReviewerApproved
            (WorkflowState::Active(data), WorkflowCommand::ReviewerApproved { reviewer_id })
                if data.planning_phase == Phase::Reviewing =>
            {
                Ok(vec![WorkflowEvent::ReviewerApproved {
                    reviewer_id,
                    approved_at: now,
                }])
            }

            // ReviewerRejected
            (
                WorkflowState::Active(data),
                WorkflowCommand::ReviewerRejected {
                    reviewer_id,
                    feedback_path,
                },
            ) if data.planning_phase == Phase::Reviewing => {
                Ok(vec![WorkflowEvent::ReviewerRejected {
                    reviewer_id,
                    feedback_path,
                    rejected_at: now,
                }])
            }

            // ReviewCycleCompleted
            (WorkflowState::Active(data), WorkflowCommand::ReviewCycleCompleted { approved })
                if data.planning_phase == Phase::Reviewing =>
            {
                Ok(vec![WorkflowEvent::ReviewCycleCompleted {
                    approved,
                    completed_at: now,
                }])
            }

            // RevisingStarted
            (
                WorkflowState::Active(data),
                WorkflowCommand::RevisingStarted { feedback_summary },
            ) if data.planning_phase == Phase::Revising => {
                Ok(vec![WorkflowEvent::RevisingStarted {
                    feedback_summary,
                    started_at: now,
                }])
            }

            // RevisionCompleted
            (WorkflowState::Active(data), WorkflowCommand::RevisionCompleted { plan_path })
                if data.planning_phase == Phase::Revising =>
            {
                Ok(vec![WorkflowEvent::RevisionCompleted {
                    plan_path,
                    completed_at: now,
                }])
            }

            // PlanningMaxIterationsReached - valid from Reviewing or Revising
            (WorkflowState::Active(_), WorkflowCommand::PlanningMaxIterationsReached) => {
                Ok(vec![WorkflowEvent::PlanningMaxIterationsReached {
                    reached_at: now,
                }])
            }

            // UserApproved - valid from AwaitingDecision or Complete
            (WorkflowState::Active(_), WorkflowCommand::UserApproved) => {
                Ok(vec![WorkflowEvent::UserApproved { approved_at: now }])
            }

            // UserRequestedImplementation - emits both request and start events
            (WorkflowState::Active(data), WorkflowCommand::UserRequestedImplementation) => {
                Ok(vec![
                    WorkflowEvent::UserRequestedImplementation { requested_at: now },
                    WorkflowEvent::ImplementationStarted {
                        max_iterations: data.max_iterations,
                        started_at: now,
                    },
                ])
            }

            // UserDeclined
            (WorkflowState::Active(_), WorkflowCommand::UserDeclined { feedback }) => {
                Ok(vec![WorkflowEvent::UserDeclined {
                    feedback,
                    declined_at: now,
                }])
            }

            // UserAborted
            (WorkflowState::Active(_), WorkflowCommand::UserAborted { reason }) => {
                Ok(vec![WorkflowEvent::UserAborted {
                    reason,
                    aborted_at: now,
                }])
            }

            // UserOverrideApproval
            (
                WorkflowState::Active(_),
                WorkflowCommand::UserOverrideApproval { override_reason },
            ) => Ok(vec![WorkflowEvent::UserOverrideApproval {
                override_reason,
                overridden_at: now,
            }]),

            // ImplementationStarted - REJECTED as direct command
            (WorkflowState::Active(_), WorkflowCommand::ImplementationStarted { .. }) => {
                Err(WorkflowError::InvalidTransition {
                    message: "ImplementationStarted is emitted by UserRequestedImplementation"
                        .to_string(),
                })
            }

            // ImplementationRoundStarted
            (
                WorkflowState::Active(data),
                WorkflowCommand::ImplementationRoundStarted { iteration },
            ) if data.implementation_state.is_some() => {
                Ok(vec![WorkflowEvent::ImplementationRoundStarted {
                    iteration,
                    started_at: now,
                }])
            }

            // ImplementationRoundCompleted
            (
                WorkflowState::Active(data),
                WorkflowCommand::ImplementationRoundCompleted {
                    iteration,
                    fingerprint,
                },
            ) if data.implementation_state.is_some() => {
                Ok(vec![WorkflowEvent::ImplementationRoundCompleted {
                    iteration,
                    fingerprint,
                    completed_at: now,
                }])
            }

            // ImplementationReviewCompleted
            (
                WorkflowState::Active(data),
                WorkflowCommand::ImplementationReviewCompleted {
                    iteration,
                    verdict,
                    feedback,
                },
            ) if data.implementation_state.is_some() => {
                Ok(vec![WorkflowEvent::ImplementationReviewCompleted {
                    iteration,
                    verdict,
                    feedback,
                    completed_at: now,
                }])
            }

            // ImplementationMaxIterationsReached
            (WorkflowState::Active(data), WorkflowCommand::ImplementationMaxIterationsReached)
                if data.implementation_state.is_some() =>
            {
                Ok(vec![WorkflowEvent::ImplementationMaxIterationsReached {
                    reached_at: now,
                }])
            }

            // ImplementationAccepted
            (WorkflowState::Active(data), WorkflowCommand::ImplementationAccepted)
                if data.implementation_state.is_some() =>
            {
                Ok(vec![WorkflowEvent::ImplementationAccepted {
                    approved_at: now,
                }])
            }

            // ImplementationDeclined
            (WorkflowState::Active(data), WorkflowCommand::ImplementationDeclined { reason })
                if data.implementation_state.is_some() =>
            {
                Ok(vec![WorkflowEvent::ImplementationDeclined {
                    reason,
                    declined_at: now,
                }])
            }

            // ImplementationCancelled
            (WorkflowState::Active(data), WorkflowCommand::ImplementationCancelled { reason })
                if data.implementation_state.is_some() =>
            {
                Ok(vec![WorkflowEvent::ImplementationCancelled {
                    reason,
                    cancelled_at: now,
                }])
            }

            // RecordAgentConversation - always valid on active aggregate
            (
                WorkflowState::Active(_),
                WorkflowCommand::RecordAgentConversation {
                    agent_id,
                    resume_strategy,
                    conversation_id,
                },
            ) => Ok(vec![WorkflowEvent::AgentConversationRecorded {
                agent_id,
                resume_strategy,
                conversation_id,
                updated_at: now,
            }]),

            // RecordInvocation - always valid on active aggregate
            (
                WorkflowState::Active(_),
                WorkflowCommand::RecordInvocation {
                    agent_id,
                    phase,
                    conversation_id,
                    resume_strategy,
                },
            ) => Ok(vec![WorkflowEvent::InvocationRecorded {
                agent_id,
                phase,
                timestamp: now,
                conversation_id,
                resume_strategy,
            }]),

            // RecordFailure - always valid on active aggregate
            (WorkflowState::Active(_), WorkflowCommand::RecordFailure { failure }) => {
                Ok(vec![WorkflowEvent::FailureRecorded {
                    failure,
                    recorded_at: now,
                }])
            }

            // AttachWorktree - always valid on active aggregate
            (WorkflowState::Active(_), WorkflowCommand::AttachWorktree { worktree_state }) => {
                Ok(vec![WorkflowEvent::WorktreeAttached { worktree_state }])
            }

            // Commands on uninitialized aggregate (except CreateWorkflow which is handled above)
            (WorkflowState::Uninitialized, _cmd) => Err(WorkflowError::NotInitialized),

            // All other combinations are invalid transitions on active aggregate
            (WorkflowState::Active(data), cmd) => {
                let cmd_name = command_name(&cmd);
                let phase = &data.planning_phase;
                Err(WorkflowError::InvalidTransition {
                    message: format!("command '{}' not valid in phase '{:?}'", cmd_name, phase),
                })
            }
        }
    }

    fn apply(&mut self, event: Self::Event) {
        match (&mut self.state, event) {
            // WorkflowCreated initializes the aggregate
            (
                WorkflowState::Uninitialized,
                WorkflowEvent::WorkflowCreated {
                    feature_name,
                    objective,
                    working_dir,
                    max_iterations,
                    plan_path,
                    feedback_path,
                    created_at,
                },
            ) => {
                self.state = WorkflowState::Active(Box::new(WorkflowData {
                    feature_name,
                    objective,
                    working_dir,
                    created_at,
                    planning_phase: Phase::Planning,
                    iteration: Iteration::first(),
                    max_iterations,
                    plan_path,
                    feedback_path,
                    last_feedback_status: None,
                    review_mode: None,
                    approval_overridden: false,
                    implementation_state: None,
                    agent_conversations: HashMap::new(),
                    invocations: Vec::new(),
                    last_failure: None,
                    failure_history: Vec::new(),
                    worktree_info: None,
                }));
            }

            // PlanningStarted
            (WorkflowState::Active(data), WorkflowEvent::PlanningStarted { .. }) => {
                data.planning_phase = Phase::Planning;
            }

            // PlanningCompleted
            (WorkflowState::Active(data), WorkflowEvent::PlanningCompleted { plan_path, .. }) => {
                data.plan_path = plan_path;
                data.planning_phase = Phase::Reviewing;
            }

            // ReviewCycleStarted
            (WorkflowState::Active(data), WorkflowEvent::ReviewCycleStarted { mode, .. }) => {
                data.planning_phase = Phase::Reviewing;
                data.review_mode = Some(mode);
            }

            // ReviewerApproved
            (WorkflowState::Active(data), WorkflowEvent::ReviewerApproved { reviewer_id, .. }) => {
                if let Some(ReviewMode::Sequential(ref mut state)) = data.review_mode {
                    state.approvals.insert(reviewer_id, state.plan_version);
                }
            }

            // ReviewerRejected
            (WorkflowState::Active(data), WorkflowEvent::ReviewerRejected { reviewer_id, .. }) => {
                if let Some(ReviewMode::Sequential(ref mut state)) = data.review_mode {
                    state.last_rejecting_reviewer = Some(reviewer_id);
                }
            }

            // ReviewCycleCompleted
            (WorkflowState::Active(data), WorkflowEvent::ReviewCycleCompleted { approved, .. }) => {
                data.planning_phase = if approved {
                    Phase::Complete
                } else {
                    Phase::Revising
                };
                data.last_feedback_status = Some(if approved {
                    FeedbackStatus::Approved
                } else {
                    FeedbackStatus::NeedsRevision
                });
            }

            // RevisingStarted
            (WorkflowState::Active(data), WorkflowEvent::RevisingStarted { .. }) => {
                data.planning_phase = Phase::Revising;
            }

            // RevisionCompleted
            (WorkflowState::Active(data), WorkflowEvent::RevisionCompleted { plan_path, .. }) => {
                data.plan_path = plan_path;
                data.iteration = data.iteration.next();
                data.planning_phase = Phase::Reviewing;
                if let Some(ReviewMode::Sequential(ref mut state)) = data.review_mode {
                    state.plan_version += 1;
                    state.approvals.clear();
                    state.accumulated_reviews.clear();
                    state.current_cycle_order.clear();
                }
            }

            // PlanningMaxIterationsReached
            (WorkflowState::Active(data), WorkflowEvent::PlanningMaxIterationsReached { .. }) => {
                data.planning_phase = Phase::AwaitingPlanningDecision;
            }

            // UserApproved
            (WorkflowState::Active(data), WorkflowEvent::UserApproved { .. }) => {
                data.planning_phase = Phase::Complete;
            }

            // UserRequestedImplementation - no state change (ImplementationStarted follows)
            (WorkflowState::Active(_), WorkflowEvent::UserRequestedImplementation { .. }) => {}

            // UserDeclined - no state change
            (WorkflowState::Active(_), WorkflowEvent::UserDeclined { .. }) => {}

            // UserAborted - no state change
            (WorkflowState::Active(_), WorkflowEvent::UserAborted { .. }) => {}

            // UserOverrideApproval
            (WorkflowState::Active(data), WorkflowEvent::UserOverrideApproval { .. }) => {
                data.approval_overridden = true;
                data.planning_phase = Phase::Complete;
            }

            // ImplementationStarted
            (
                WorkflowState::Active(data),
                WorkflowEvent::ImplementationStarted { max_iterations, .. },
            ) => {
                data.implementation_state = Some(ImplementationPhaseState::new(max_iterations));
            }

            // ImplementationRoundStarted
            (
                WorkflowState::Active(data),
                WorkflowEvent::ImplementationRoundStarted { iteration, .. },
            ) => {
                if let Some(ref mut state) = data.implementation_state {
                    state.phase = ImplementationPhase::Implementing;
                    state.iteration = iteration;
                }
            }

            // ImplementationRoundCompleted
            (WorkflowState::Active(_), WorkflowEvent::ImplementationRoundCompleted { .. }) => {}

            // ImplementationReviewCompleted
            (
                WorkflowState::Active(data),
                WorkflowEvent::ImplementationReviewCompleted {
                    verdict, feedback, ..
                },
            ) => {
                if let Some(ref mut state) = data.implementation_state {
                    state.phase = ImplementationPhase::ImplementationReview;
                    state.last_verdict = Some(verdict);
                    state.last_feedback = feedback;
                }
            }

            // ImplementationMaxIterationsReached
            (
                WorkflowState::Active(data),
                WorkflowEvent::ImplementationMaxIterationsReached { .. },
            ) => {
                if let Some(ref mut state) = data.implementation_state {
                    state.phase = ImplementationPhase::AwaitingDecision;
                }
            }

            // ImplementationAccepted, ImplementationDeclined, ImplementationCancelled
            (WorkflowState::Active(data), WorkflowEvent::ImplementationAccepted { .. })
            | (WorkflowState::Active(data), WorkflowEvent::ImplementationDeclined { .. })
            | (WorkflowState::Active(data), WorkflowEvent::ImplementationCancelled { .. }) => {
                if let Some(ref mut state) = data.implementation_state {
                    state.phase = ImplementationPhase::Complete;
                }
            }

            // AgentConversationRecorded
            (
                WorkflowState::Active(data),
                WorkflowEvent::AgentConversationRecorded {
                    agent_id,
                    resume_strategy,
                    conversation_id,
                    updated_at,
                },
            ) => {
                data.agent_conversations.insert(
                    agent_id,
                    AgentConversationState {
                        resume_strategy,
                        conversation_id,
                        last_used_at: updated_at,
                    },
                );
            }

            // InvocationRecorded
            (
                WorkflowState::Active(data),
                WorkflowEvent::InvocationRecorded {
                    agent_id,
                    phase,
                    timestamp,
                    conversation_id,
                    resume_strategy,
                },
            ) => {
                data.invocations.push(InvocationRecord {
                    agent: agent_id,
                    phase,
                    timestamp,
                    conversation_id,
                    resume_strategy,
                });
            }

            // FailureRecorded
            (WorkflowState::Active(data), WorkflowEvent::FailureRecorded { failure, .. }) => {
                data.last_failure = Some(failure.clone());
                data.failure_history.push(failure);
                if data.failure_history.len() > MAX_FAILURE_HISTORY {
                    let excess = data.failure_history.len() - MAX_FAILURE_HISTORY;
                    data.failure_history.drain(0..excess);
                }
            }

            // WorktreeAttached
            (WorkflowState::Active(data), WorkflowEvent::WorktreeAttached { worktree_state }) => {
                data.worktree_info = Some(worktree_state);
            }

            // Ignore events on wrong state (shouldn't happen with correct event sourcing)
            _ => {}
        }
    }
}

/// Extracts a human-readable name from a command for error messages.
fn command_name(cmd: &WorkflowCommand) -> &'static str {
    match cmd {
        WorkflowCommand::CreateWorkflow { .. } => "CreateWorkflow",
        WorkflowCommand::StartPlanning => "StartPlanning",
        WorkflowCommand::PlanningCompleted { .. } => "PlanningCompleted",
        WorkflowCommand::ReviewCycleStarted { .. } => "ReviewCycleStarted",
        WorkflowCommand::ReviewerApproved { .. } => "ReviewerApproved",
        WorkflowCommand::ReviewerRejected { .. } => "ReviewerRejected",
        WorkflowCommand::ReviewCycleCompleted { .. } => "ReviewCycleCompleted",
        WorkflowCommand::RevisingStarted { .. } => "RevisingStarted",
        WorkflowCommand::RevisionCompleted { .. } => "RevisionCompleted",
        WorkflowCommand::PlanningMaxIterationsReached => "PlanningMaxIterationsReached",
        WorkflowCommand::UserApproved => "UserApproved",
        WorkflowCommand::UserRequestedImplementation => "UserRequestedImplementation",
        WorkflowCommand::UserDeclined { .. } => "UserDeclined",
        WorkflowCommand::UserAborted { .. } => "UserAborted",
        WorkflowCommand::UserOverrideApproval { .. } => "UserOverrideApproval",
        WorkflowCommand::ImplementationStarted { .. } => "ImplementationStarted",
        WorkflowCommand::ImplementationRoundStarted { .. } => "ImplementationRoundStarted",
        WorkflowCommand::ImplementationRoundCompleted { .. } => "ImplementationRoundCompleted",
        WorkflowCommand::ImplementationReviewCompleted { .. } => "ImplementationReviewCompleted",
        WorkflowCommand::ImplementationMaxIterationsReached => "ImplementationMaxIterationsReached",
        WorkflowCommand::ImplementationAccepted => "ImplementationAccepted",
        WorkflowCommand::ImplementationDeclined { .. } => "ImplementationDeclined",
        WorkflowCommand::ImplementationCancelled { .. } => "ImplementationCancelled",
        WorkflowCommand::RecordAgentConversation { .. } => "RecordAgentConversation",
        WorkflowCommand::RecordInvocation { .. } => "RecordInvocation",
        WorkflowCommand::RecordFailure { .. } => "RecordFailure",
        WorkflowCommand::AttachWorktree { .. } => "AttachWorktree",
    }
}

#[cfg(test)]
#[path = "../tests/aggregate_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/aggregate_impl_tests.rs"]
mod impl_tests;
