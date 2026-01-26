//! Workflow view projection for UI and query purposes.
//!
//! The WorkflowView is derived from WorkflowEvent only (no direct mutation)
//! and contains only the data required for UI, session tracking, and resume.

use crate::domain::cqrs::WorkflowAggregate;
use crate::domain::failure::{FailureContext, MAX_FAILURE_HISTORY};
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentConversationState, AgentId, FeatureName, FeedbackPath, FeedbackStatus,
    ImplementationPhase, ImplementationPhaseState, InvocationRecord, Iteration, MaxIterations,
    Objective, PlanPath, PlanningPhase, UiMode, WorkflowId, WorkingDir, WorktreeState,
};
use crate::domain::WorkflowEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Read-only view of workflow state derived from events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowView {
    pub workflow_id: Option<WorkflowId>,
    pub feature_name: Option<FeatureName>,
    pub objective: Option<Objective>,
    pub working_dir: Option<WorkingDir>,
    pub plan_path: Option<PlanPath>,
    pub feedback_path: Option<FeedbackPath>,
    pub planning_phase: Option<PlanningPhase>,
    pub iteration: Option<Iteration>,
    pub max_iterations: Option<MaxIterations>,
    pub last_feedback_status: Option<FeedbackStatus>,
    pub review_mode: Option<ReviewMode>,
    pub implementation_state: Option<ImplementationPhaseState>,
    pub agent_conversations: HashMap<AgentId, AgentConversationState>,
    pub invocations: Vec<InvocationRecord>,
    pub last_failure: Option<FailureContext>,
    pub failure_history: Vec<FailureContext>,
    pub worktree_info: Option<WorktreeState>,
    pub approval_overridden: bool,
    pub last_event_sequence: u64,
}

impl WorkflowView {
    /// Apply an event to update the view.
    pub fn apply_event(&mut self, aggregate_id: &str, event: &WorkflowEvent, sequence: u64) {
        // Parse aggregate_id as UUID - log warning on invalid format
        match Uuid::parse_str(aggregate_id) {
            Ok(uuid) => self.workflow_id = Some(WorkflowId(uuid)),
            Err(e) => tracing::warn!("Invalid aggregate ID '{}': {}", aggregate_id, e),
        }
        self.last_event_sequence = sequence;

        match event {
            WorkflowEvent::WorkflowCreated {
                feature_name,
                objective,
                working_dir,
                max_iterations,
                plan_path,
                feedback_path,
                ..
            } => {
                self.feature_name = Some(feature_name.clone());
                self.objective = Some(objective.clone());
                self.working_dir = Some(working_dir.clone());
                self.max_iterations = Some(*max_iterations);
                self.plan_path = Some(plan_path.clone());
                self.feedback_path = Some(feedback_path.clone());
                self.planning_phase = Some(PlanningPhase::Planning);
                self.iteration = Some(Iteration::first());
                self.review_mode = None;
                self.last_feedback_status = None;
                self.approval_overridden = false;
                self.implementation_state = None;
                self.agent_conversations.clear();
                self.invocations.clear();
                self.last_failure = None;
                self.failure_history.clear();
                self.worktree_info = None;
            }

            WorkflowEvent::PlanningStarted { .. } => {
                self.planning_phase = Some(PlanningPhase::Planning);
            }

            WorkflowEvent::PlanningCompleted { plan_path, .. } => {
                self.plan_path = Some(plan_path.clone());
                self.planning_phase = Some(PlanningPhase::Reviewing);
            }

            WorkflowEvent::ReviewCycleStarted { mode, .. } => {
                self.review_mode = Some(mode.clone());
                self.planning_phase = Some(PlanningPhase::Reviewing);
            }

            WorkflowEvent::ReviewerApproved { reviewer_id, .. } => {
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state
                        .approvals
                        .insert(reviewer_id.clone(), state.plan_version);
                }
            }

            WorkflowEvent::ReviewerRejected { reviewer_id, .. } => {
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state.last_rejecting_reviewer = Some(reviewer_id.clone());
                }
            }

            WorkflowEvent::ReviewCycleCompleted { approved, .. } => {
                self.planning_phase = Some(if *approved {
                    PlanningPhase::Complete
                } else {
                    PlanningPhase::Revising
                });
                self.last_feedback_status = Some(if *approved {
                    FeedbackStatus::Approved
                } else {
                    FeedbackStatus::NeedsRevision
                });
            }

            WorkflowEvent::RevisingStarted { .. } => {
                self.planning_phase = Some(PlanningPhase::Revising);
            }

            WorkflowEvent::RevisionCompleted { plan_path, .. } => {
                self.plan_path = Some(plan_path.clone());
                self.planning_phase = Some(PlanningPhase::Reviewing);
                let current = self
                    .iteration
                    .expect("WorkflowView must be initialized before RevisionCompleted");
                self.iteration = Some(current.next());
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state.plan_version += 1;
                    state.approvals.clear();
                    state.accumulated_reviews.clear();
                    state.current_cycle_order.clear();
                }
            }

            WorkflowEvent::PlanningMaxIterationsReached { .. } => {
                self.planning_phase = Some(PlanningPhase::AwaitingDecision);
            }

            WorkflowEvent::UserApproved { .. } => {
                self.planning_phase = Some(PlanningPhase::Complete);
            }

            WorkflowEvent::UserRequestedImplementation { .. } => {
                // No state change - ImplementationStarted follows
            }

            WorkflowEvent::UserOverrideApproval { .. } => {
                self.approval_overridden = true;
                self.planning_phase = Some(PlanningPhase::Complete);
            }

            WorkflowEvent::UserDeclined { .. } | WorkflowEvent::UserAborted { .. } => {
                // No state change
            }

            WorkflowEvent::ImplementationStarted { max_iterations, .. } => {
                self.implementation_state = Some(ImplementationPhaseState::new(*max_iterations));
            }

            WorkflowEvent::ImplementationRoundStarted { iteration, .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.phase = ImplementationPhase::Implementing;
                    state.iteration = *iteration;
                }
            }

            WorkflowEvent::ImplementationRoundCompleted { .. } => {
                // No state change
            }

            WorkflowEvent::ImplementationReviewCompleted {
                verdict, feedback, ..
            } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.phase = ImplementationPhase::ImplementationReview;
                    state.last_verdict = Some(*verdict);
                    state.last_feedback = feedback.clone();
                }
            }

            WorkflowEvent::ImplementationMaxIterationsReached { .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.phase = ImplementationPhase::AwaitingDecision;
                }
            }

            WorkflowEvent::ImplementationAccepted { .. }
            | WorkflowEvent::ImplementationDeclined { .. }
            | WorkflowEvent::ImplementationCancelled { .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.phase = ImplementationPhase::Complete;
                }
            }

            WorkflowEvent::AgentConversationRecorded {
                agent_id,
                resume_strategy,
                conversation_id,
                updated_at,
            } => {
                self.agent_conversations.insert(
                    agent_id.clone(),
                    AgentConversationState {
                        resume_strategy: *resume_strategy,
                        conversation_id: conversation_id.clone(),
                        last_used_at: *updated_at,
                    },
                );
            }

            WorkflowEvent::InvocationRecorded {
                agent_id,
                phase,
                timestamp,
                conversation_id,
                resume_strategy,
            } => {
                self.invocations.push(InvocationRecord {
                    agent: agent_id.clone(),
                    phase: *phase,
                    timestamp: *timestamp,
                    conversation_id: conversation_id.clone(),
                    resume_strategy: *resume_strategy,
                });
            }

            WorkflowEvent::FailureRecorded { failure, .. } => {
                self.last_failure = Some(failure.clone());
                self.failure_history.push(failure.clone());
                if self.failure_history.len() > MAX_FAILURE_HISTORY {
                    let excess = self.failure_history.len() - MAX_FAILURE_HISTORY;
                    self.failure_history.drain(0..excess);
                }
            }

            WorkflowEvent::WorktreeAttached { worktree_state } => {
                self.worktree_info = Some(worktree_state.clone());
            }
        }
    }

    /// Returns the current UI mode based on implementation state.
    pub fn ui_mode(&self) -> UiMode {
        match &self.implementation_state {
            Some(impl_state) if impl_state.phase != ImplementationPhase::Complete => {
                UiMode::Implementation
            }
            _ => UiMode::Planning,
        }
    }

    /// Returns true if there's an active failure requiring recovery.
    pub fn has_failure(&self) -> bool {
        self.last_failure.is_some()
    }

    /// Returns true if the workflow should continue (not complete and within iteration limits).
    pub fn should_continue(&self) -> bool {
        if self.planning_phase == Some(PlanningPhase::Complete) {
            return false;
        }
        match (self.iteration, self.max_iterations) {
            (Some(iter), Some(max)) => iter.0 <= max.0,
            _ => false,
        }
    }
}

/// Serializable wrapper for event envelopes used in RPC and broadcasting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEventEnvelope {
    pub aggregate_id: String,
    pub sequence: u64,
    pub event: WorkflowEvent,
}

impl From<&cqrs_es::EventEnvelope<WorkflowAggregate>> for WorkflowEventEnvelope {
    fn from(source: &cqrs_es::EventEnvelope<WorkflowAggregate>) -> Self {
        Self {
            aggregate_id: source.aggregate_id.clone(),
            sequence: source.sequence as u64,
            event: source.payload.clone(),
        }
    }
}
