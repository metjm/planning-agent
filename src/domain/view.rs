//! Workflow view projection for UI and query purposes.
//!
//! The WorkflowView is derived from WorkflowEvent only (no direct mutation)
//! and contains only the data required for UI, session tracking, and resume.

use crate::domain::cqrs::WorkflowAggregate;
use crate::domain::failure::{FailureContext, MAX_FAILURE_HISTORY};
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentConversationState, AgentId, AwaitingDecisionReason, FeatureName, FeedbackPath,
    FeedbackStatus, ImplementationPhase, ImplementationPhaseState, InvocationRecord, Iteration,
    MaxIterations, Objective, Phase, PlanPath, ReviewerResult, UiMode, WorkflowId, WorkingDir,
    WorktreeState,
};
use crate::domain::WorkflowEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Read-only view of workflow state derived from events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowView {
    workflow_id: Option<WorkflowId>,
    feature_name: Option<FeatureName>,
    objective: Option<Objective>,
    working_dir: Option<WorkingDir>,
    plan_path: Option<PlanPath>,
    feedback_path: Option<FeedbackPath>,
    planning_phase: Option<Phase>,
    iteration: Option<Iteration>,
    max_iterations: Option<MaxIterations>,
    last_feedback_status: Option<FeedbackStatus>,
    review_mode: Option<ReviewMode>,
    implementation_state: Option<ImplementationPhaseState>,
    agent_conversations: HashMap<AgentId, AgentConversationState>,
    invocations: Vec<InvocationRecord>,
    last_failure: Option<FailureContext>,
    failure_history: Vec<FailureContext>,
    worktree_info: Option<WorktreeState>,
    approval_overridden: bool,
    last_event_sequence: u64,
    /// Review results from the current review cycle.
    /// Cleared when a new review cycle starts or revision completes.
    #[serde(default)]
    current_cycle_reviews: Vec<ReviewerResult>,
    /// Accumulated user feedback from interrupts/declines.
    /// Used to provide context to the planning agent on restart.
    #[serde(default)]
    user_feedback_history: Vec<String>,
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
                self.planning_phase = Some(Phase::Planning);
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
                self.planning_phase = Some(Phase::Planning);
            }

            WorkflowEvent::PlanningCompleted { plan_path, .. } => {
                self.plan_path = Some(plan_path.clone());
                self.planning_phase = Some(Phase::Reviewing);
            }

            WorkflowEvent::ReviewCycleStarted { mode, .. } => {
                self.review_mode = Some(mode.clone());
                self.planning_phase = Some(Phase::Reviewing);
                // Clear previous cycle's reviews when starting a new cycle
                self.current_cycle_reviews.clear();
            }

            WorkflowEvent::ReviewerApproved { reviewer_id, .. } => {
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state.record_approval_simple(reviewer_id.clone());
                    state.advance_to_next_reviewer();
                }
                // Track this reviewer's approval for resume
                self.current_cycle_reviews
                    .push(ReviewerResult::approved(reviewer_id.clone()));
            }

            WorkflowEvent::ReviewerRejected {
                reviewer_id,
                feedback_path,
                ..
            } => {
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state.record_rejection(reviewer_id.as_str());
                }
                // Track this reviewer's rejection with feedback path for resume
                self.current_cycle_reviews.push(ReviewerResult::rejected(
                    reviewer_id.clone(),
                    feedback_path.clone(),
                ));
            }

            WorkflowEvent::ReviewCycleCompleted { approved, .. } => {
                self.planning_phase = Some(if *approved {
                    Phase::Complete
                } else {
                    Phase::Revising
                });
                self.last_feedback_status = Some(if *approved {
                    FeedbackStatus::Approved
                } else {
                    FeedbackStatus::NeedsRevision
                });
            }

            WorkflowEvent::RevisingStarted { .. } => {
                self.planning_phase = Some(Phase::Revising);
            }

            WorkflowEvent::RevisionCompleted { plan_path, .. } => {
                self.plan_path = Some(plan_path.clone());
                self.planning_phase = Some(Phase::Reviewing);
                let current = self
                    .iteration
                    .expect("WorkflowView must be initialized before RevisionCompleted");
                self.iteration = Some(current.next());
                if let Some(ReviewMode::Sequential(ref mut state)) = self.review_mode {
                    state.increment_version();
                    state.clear_cycle_order();
                }
                // Clear reviews after revision - new review cycle will start
                self.current_cycle_reviews.clear();
            }

            WorkflowEvent::PlanningMaxIterationsReached { .. } => {
                self.planning_phase = Some(Phase::AwaitingPlanningDecision);
            }

            WorkflowEvent::MaxIterationsExtended { new_max, .. } => {
                self.max_iterations = Some(*new_max);
            }

            WorkflowEvent::UserApproved { .. } => {
                self.planning_phase = Some(Phase::Complete);
            }

            WorkflowEvent::UserRequestedImplementation { .. } => {
                // No state change - ImplementationStarted follows
            }

            WorkflowEvent::UserOverrideApproval { .. } => {
                self.approval_overridden = true;
                self.planning_phase = Some(Phase::Complete);
            }

            WorkflowEvent::UserDeclined { feedback, .. } => {
                // Accumulate feedback for planning agent context
                if !feedback.is_empty() {
                    self.user_feedback_history.push(feedback.clone());
                }
            }

            WorkflowEvent::UserAborted { .. } => {
                // No state change
            }

            WorkflowEvent::ImplementationStarted { max_iterations, .. } => {
                self.implementation_state = Some(ImplementationPhaseState::new(*max_iterations));
            }

            WorkflowEvent::ImplementationRoundStarted { iteration, .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.set_phase(ImplementationPhase::Implementing);
                    state.set_iteration(*iteration);
                    state.set_decision_reason(None); // Clear stale decision reason
                }
            }

            WorkflowEvent::ImplementationRoundCompleted { .. } => {
                // No state change
            }

            WorkflowEvent::ImplementationReviewCompleted {
                verdict, feedback, ..
            } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.set_phase(ImplementationPhase::ImplementationReview);
                    state.set_verdict(Some(*verdict));
                    state.set_feedback(feedback.clone());
                }
            }

            WorkflowEvent::ImplementationMaxIterationsReached { .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.set_phase(ImplementationPhase::AwaitingDecision);
                    state.set_decision_reason(Some(AwaitingDecisionReason::MaxIterationsReached));
                }
            }

            WorkflowEvent::ImplementationNoChanges { .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.set_phase(ImplementationPhase::AwaitingDecision);
                    state.set_decision_reason(Some(AwaitingDecisionReason::NoChanges));
                }
            }

            WorkflowEvent::ImplementationAccepted { .. }
            | WorkflowEvent::ImplementationDeclined { .. }
            | WorkflowEvent::ImplementationCancelled { .. } => {
                if let Some(ref mut state) = self.implementation_state {
                    state.set_phase(ImplementationPhase::Complete);
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
                    AgentConversationState::new(
                        *resume_strategy,
                        conversation_id.clone(),
                        *updated_at,
                    ),
                );
            }

            WorkflowEvent::InvocationRecorded {
                agent_id,
                phase,
                timestamp,
                conversation_id,
                resume_strategy,
            } => {
                self.invocations.push(InvocationRecord::new(
                    agent_id.clone(),
                    *phase,
                    *timestamp,
                    conversation_id.clone(),
                    *resume_strategy,
                ));
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

    /// Returns a reference to the agent conversations map.
    pub fn agent_conversations(&self) -> &HashMap<AgentId, AgentConversationState> {
        &self.agent_conversations
    }

    /// Returns a reference to the invocation records.
    pub fn invocations(&self) -> &[InvocationRecord] {
        &self.invocations
    }

    /// Returns a reference to the failure history.
    pub fn failure_history(&self) -> &[FailureContext] {
        &self.failure_history
    }

    /// Returns the workflow ID.
    pub fn workflow_id(&self) -> Option<&WorkflowId> {
        self.workflow_id.as_ref()
    }

    /// Returns the feature name.
    pub fn feature_name(&self) -> Option<&FeatureName> {
        self.feature_name.as_ref()
    }

    /// Returns the objective.
    pub fn objective(&self) -> Option<&Objective> {
        self.objective.as_ref()
    }

    /// Returns the working directory.
    pub fn working_dir(&self) -> Option<&WorkingDir> {
        self.working_dir.as_ref()
    }

    /// Returns the plan path.
    pub fn plan_path(&self) -> Option<&PlanPath> {
        self.plan_path.as_ref()
    }

    /// Returns the feedback path.
    pub fn feedback_path(&self) -> Option<&FeedbackPath> {
        self.feedback_path.as_ref()
    }

    /// Returns the current planning phase.
    pub fn planning_phase(&self) -> Option<Phase> {
        self.planning_phase
    }

    /// Returns the current iteration.
    pub fn iteration(&self) -> Option<Iteration> {
        self.iteration
    }

    /// Returns the maximum iterations.
    pub fn max_iterations(&self) -> Option<MaxIterations> {
        self.max_iterations
    }

    /// Returns the last feedback status.
    pub fn last_feedback_status(&self) -> Option<FeedbackStatus> {
        self.last_feedback_status
    }

    /// Returns the review mode.
    pub fn review_mode(&self) -> Option<&ReviewMode> {
        self.review_mode.as_ref()
    }

    /// Returns the implementation state.
    pub fn implementation_state(&self) -> Option<&ImplementationPhaseState> {
        self.implementation_state.as_ref()
    }

    /// Returns the last failure.
    pub fn last_failure(&self) -> Option<&FailureContext> {
        self.last_failure.as_ref()
    }

    /// Returns the worktree info.
    pub fn worktree_info(&self) -> Option<&WorktreeState> {
        self.worktree_info.as_ref()
    }

    /// Returns whether approval was overridden.
    pub fn approval_overridden(&self) -> bool {
        self.approval_overridden
    }

    /// Returns the last event sequence number.
    pub fn last_event_sequence(&self) -> u64 {
        self.last_event_sequence
    }

    /// Returns the review results from the current review cycle.
    /// This data survives session resume and is used by the revising phase.
    pub fn current_cycle_reviews(&self) -> &[ReviewerResult] {
        &self.current_cycle_reviews
    }

    /// Returns accumulated user feedback from interrupts/declines.
    pub fn user_feedback_history(&self) -> &[String] {
        &self.user_feedback_history
    }

    /// Returns the current UI mode based on implementation state.
    pub fn ui_mode(&self) -> UiMode {
        match &self.implementation_state {
            Some(impl_state) if impl_state.phase() != ImplementationPhase::Complete => {
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
        if self.planning_phase == Some(Phase::Complete) {
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

#[cfg(test)]
#[path = "tests/view_tests.rs"]
mod tests;
