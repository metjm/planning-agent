//! Centralized state machine for workflow state management.
//!
//! This module provides the ONLY place where state transitions happen.
//! The state machine owns the state, validates commands, emits events,
//! and broadcasts snapshots to subscribers via a watch channel.

mod commands;
mod events;
mod snapshot;

pub use commands::StateCommand;
pub use events::StateEvent;
pub use snapshot::StateSnapshot;

use crate::app::failure::{FailureContext, FailureKind};
use crate::state::{Phase, SequentialReviewState, State};
use crate::structured_logger::StructuredLogger;
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::watch;

/// The ONLY place state transitions happen.
/// Owns the state, validates commands, emits events, broadcasts snapshots.
pub struct WorkflowStateMachine {
    state: State,
    snapshot_tx: watch::Sender<StateSnapshot>,
    logger: Arc<StructuredLogger>,
    seq: u64,
}

impl WorkflowStateMachine {
    /// Creates a new state machine with the given initial state.
    ///
    /// Returns the state machine and a watch receiver for state snapshots.
    /// TUI should poll this receiver for state updates.
    pub fn new(
        initial_state: State,
        logger: Arc<StructuredLogger>,
    ) -> (Self, watch::Receiver<StateSnapshot>) {
        let snapshot = StateSnapshot::from(&initial_state);
        let (snapshot_tx, snapshot_rx) = watch::channel(snapshot);

        let machine = Self {
            state: initial_state,
            snapshot_tx,
            logger,
            seq: 0,
        };

        (machine, snapshot_rx)
    }

    /// All mutations go through this single method.
    /// Returns events for logging; broadcasts snapshot automatically.
    pub fn apply(&mut self, command: StateCommand) -> Result<Vec<StateEvent>> {
        self.seq += 1;

        // Log command receipt
        self.logger.log_command(self.seq, &command);

        // Validate and apply
        let events = self.apply_internal(command)?;

        // Log events
        for event in &events {
            self.logger.log_event(self.seq, event);
        }

        // Update timestamp and broadcast snapshot
        self.state.set_updated_at();
        let snapshot = StateSnapshot::from(&self.state);
        let _ = self.snapshot_tx.send(snapshot);

        Ok(events)
    }

    fn apply_internal(&mut self, command: StateCommand) -> Result<Vec<StateEvent>> {
        use StateCommand::*;
        use StateEvent::*;

        match command {
            StartPlanning => {
                // Validate: can only start planning from initial state or after restart
                if self.state.phase != Phase::Planning {
                    bail!("Cannot start planning from phase {:?}", self.state.phase);
                }
                // No state change needed - already in Planning
                Ok(vec![])
            }

            CompletePlanning { plan_path } => {
                if self.state.phase != Phase::Planning {
                    bail!("Cannot complete planning from phase {:?}", self.state.phase);
                }
                let from = self.state.phase.clone();
                self.state.plan_file = plan_path;
                self.state.transition(Phase::Reviewing)?;
                Ok(vec![PhaseChanged {
                    from,
                    to: self.state.phase.clone(),
                }])
            }

            StartReviewing { reviewer_id } => {
                if self.state.phase != Phase::Reviewing {
                    bail!("Cannot start reviewing from phase {:?}", self.state.phase);
                }
                Ok(vec![ReviewerStatusChanged {
                    reviewer_id,
                    approved: false,
                }])
            }

            ReviewerApproved { reviewer_id } => Ok(vec![ReviewerStatusChanged {
                reviewer_id,
                approved: true,
            }]),

            ReviewerRejected {
                reviewer_id,
                feedback_path,
            } => {
                self.state.feedback_file = feedback_path;
                Ok(vec![ReviewerStatusChanged {
                    reviewer_id,
                    approved: false,
                }])
            }

            AllReviewersComplete { approved } => {
                if self.state.phase != Phase::Reviewing {
                    bail!("AllReviewersComplete only valid in Reviewing phase");
                }
                let from = self.state.phase.clone();
                if approved {
                    self.state.transition(Phase::Complete)?;
                } else {
                    self.state.transition(Phase::Revising)?;
                }
                Ok(vec![PhaseChanged {
                    from,
                    to: self.state.phase.clone(),
                }])
            }

            StartRevising {
                feedback_content: _,
            } => {
                if self.state.phase != Phase::Revising {
                    bail!("Cannot start revising from phase {:?}", self.state.phase);
                }
                // Feedback content is passed to the revising agent, not stored in state
                Ok(vec![])
            }

            CompleteRevising => {
                if self.state.phase != Phase::Revising {
                    bail!("Cannot complete revising from phase {:?}", self.state.phase);
                }
                let from = self.state.phase.clone();
                self.state.iteration += 1;
                self.state
                    .update_feedback_for_iteration(self.state.iteration);
                self.state.transition(Phase::Reviewing)?;
                Ok(vec![
                    IterationIncremented {
                        new_value: self.state.iteration,
                    },
                    PhaseChanged {
                        from,
                        to: self.state.phase.clone(),
                    },
                ])
            }

            MarkComplete => {
                if self.state.phase == Phase::Complete {
                    return Ok(vec![]); // Already complete, no-op
                }
                if self.state.phase != Phase::Reviewing {
                    bail!("Can only mark complete from Reviewing phase");
                }
                let from = self.state.phase.clone();
                self.state.transition(Phase::Complete)?;
                Ok(vec![
                    PhaseChanged {
                        from,
                        to: self.state.phase.clone(),
                    },
                    WorkflowComplete {
                        approved: true,
                        override_used: false,
                    },
                ])
            }

            UserApprove => Ok(vec![WorkflowComplete {
                approved: true,
                override_used: false,
            }]),

            UserRequestImplementation => Ok(vec![WorkflowComplete {
                approved: true,
                override_used: false,
            }]),

            UserDecline { feedback } => {
                // User declined - workflow will handle restart
                let preview = if feedback.chars().count() > 50 {
                    format!("{}...", feedback.chars().take(50).collect::<String>())
                } else {
                    feedback
                };
                Ok(vec![WorkflowRestarted {
                    feedback_preview: preview,
                }])
            }

            UserAbort { reason } => Ok(vec![ErrorOccurred { error: reason }]),

            UserOverrideApproval => {
                // User override bypasses normal validation - this is intentional.
                // The override flag is set to track that normal review was skipped.
                self.state.approval_overridden = true;
                let from = self.state.phase.clone();
                if self.state.phase != Phase::Complete {
                    // Direct assignment is intentional here - override bypasses
                    // normal transition rules. We can't use transition() because
                    // Planning->Complete is not a valid normal transition.
                    self.state.phase = Phase::Complete;
                }
                Ok(vec![
                    PhaseChanged {
                        from,
                        to: Phase::Complete,
                    },
                    WorkflowComplete {
                        approved: true,
                        override_used: true,
                    },
                ])
            }

            IncrementIteration => {
                self.state.iteration += 1;
                self.state
                    .update_feedback_for_iteration(self.state.iteration);
                Ok(vec![IterationIncremented {
                    new_value: self.state.iteration,
                }])
            }

            ExtendMaxIterations => {
                self.state.max_iterations += 1;
                Ok(vec![])
            }

            UpdateAgentConversation {
                agent,
                conversation_id,
            } => {
                self.state
                    .update_agent_conversation_id(&agent, conversation_id);
                Ok(vec![AgentConversationUpdated { agent }])
            }

            RecordInvocation { agent, phase } => {
                self.state.record_invocation(&agent, &phase);
                Ok(vec![InvocationRecorded { agent, phase }])
            }

            AgentFailed { agent_id, error } => {
                // Use FailureContext::new() factory method with correct field types
                // FailureKind::Unknown wraps arbitrary error strings
                let failure = FailureContext::new(
                    FailureKind::Unknown(error.clone()),
                    self.state.phase.clone(), // Phase enum, not String
                    Some(agent_id),           // agent_name: Option<String>
                    3,                        // max_retries (could come from config)
                );
                // Delegate to State::set_failure() which handles history management
                // This avoids duplicating the failure history trimming logic
                self.state.set_failure(failure);
                Ok(vec![ErrorOccurred { error }])
            }

            ClearFailure => {
                // Delegate to State::clear_failure() for consistency
                self.state.clear_failure();
                Ok(vec![ErrorCleared])
            }

            InitSequentialReview => {
                // Use SequentialReviewState::new() factory which properly initializes
                // all fields: current_reviewer_index, plan_version, approvals,
                // accumulated_reviews, reviewer_run_counts, current_cycle_order,
                // last_rejecting_reviewer
                self.state.sequential_review = Some(SequentialReviewState::new());
                Ok(vec![])
            }

            ClearSequentialReview => {
                self.state.sequential_review = None;
                Ok(vec![])
            }

            AdvanceSequentialReviewer => {
                if let Some(ref mut seq_state) = self.state.sequential_review {
                    seq_state.current_reviewer_index += 1;
                }
                Ok(vec![])
            }

            RestartWithFeedback { feedback } => {
                // Reset to Planning phase - this is a user-initiated restart.
                // Direct assignment is intentional: restart bypasses normal
                // transition rules (e.g., Complete->Planning is not normally valid).
                // Note: iteration is intentionally preserved - user feedback refines
                // the current iteration rather than starting fresh. This also ensures
                // max_iterations is properly enforced.
                let from = self.state.phase.clone();
                self.state.phase = Phase::Planning;
                // Append feedback to objective
                self.state.objective =
                    format!("{}\n\n## User Feedback\n{}", self.state.objective, feedback);
                let preview = if feedback.chars().count() > 50 {
                    format!("{}...", feedback.chars().take(50).collect::<String>())
                } else {
                    feedback
                };
                Ok(vec![
                    PhaseChanged {
                        from,
                        to: Phase::Planning,
                    },
                    WorkflowRestarted {
                        feedback_preview: preview,
                    },
                ])
            }
        }
    }

    /// Returns immutable reference to current state (for saving to disk).
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Returns mutable reference to current state (for direct manipulation when needed).
    /// Use with caution - prefer using commands where possible.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Save state to disk atomically.
    pub fn save(&self, state_path: &std::path::Path) -> Result<()> {
        self.state.save_atomic(state_path)
    }

    /// Broadcasts the current state snapshot to all watchers.
    /// Useful for ensuring TUI has latest state after direct state mutations.
    pub fn broadcast_snapshot(&self) {
        let snapshot = StateSnapshot::from(&self.state);
        let _ = self.snapshot_tx.send(snapshot);
    }
}

#[cfg(test)]
mod tests;
