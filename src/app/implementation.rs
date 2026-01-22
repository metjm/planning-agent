//! Implementation orchestrator for the JSON-mode implementation workflow.
//!
//! This module provides the `run_implementation_workflow` function that manages
//! the implementation -> review loop until approval or max iterations.

use crate::change_fingerprint::compute_change_fingerprint;
use crate::config::WorkflowConfig;
use crate::phases::implementation::run_implementation_phase;
use crate::phases::implementation_review::run_implementation_review_phase;
use crate::phases::implementing_conversation_key;
use crate::phases::verdict::VerificationVerdictResult;
use crate::session_logger::SessionLogger;
use crate::state::{ImplementationPhase, ImplementationPhaseState, ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

/// Result of the implementation workflow.
#[derive(Debug, Clone)]
pub enum ImplementationWorkflowResult {
    /// Implementation was approved
    Approved,
    /// Implementation failed after max iterations
    Failed {
        iterations_used: u32,
        last_feedback: Option<String>,
    },
    /// Implementation was cancelled by user
    Cancelled { iterations_used: u32 },
    /// No changes were detected between iterations (circuit breaker)
    NoChanges { iterations_used: u32 },
}

/// Runs the implementation workflow loop.
///
/// This runs the implement/review cycle until either:
/// - Review passes (returns ImplementationWorkflowResult::Approved)
/// - Max iterations reached (returns ImplementationWorkflowResult::Failed)
/// - User cancels (returns ImplementationWorkflowResult::Cancelled)
/// - No changes detected (returns ImplementationWorkflowResult::NoChanges)
///
/// # Arguments
/// * `state` - The current workflow state (will be modified)
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory for implementation
/// * `session_sender` - Channel to send session events
/// * `session_logger` - Logger for the session
/// * `initial_feedback` - Optional initial feedback to start with
pub async fn run_implementation_workflow(
    state: &mut State,
    config: &WorkflowConfig,
    working_dir: &Path,
    session_sender: SessionEventSender,
    session_logger: Arc<SessionLogger>,
    initial_feedback: Option<String>,
) -> Result<ImplementationWorkflowResult> {
    // Validate config
    let impl_config = &config.implementation;
    if !impl_config.enabled {
        anyhow::bail!("Implementation is disabled in config");
    }

    let max_iterations = impl_config.max_iterations;

    // Initialize implementation state if not present
    if state.implementation_state.is_none() {
        state.implementation_state = Some(ImplementationPhaseState::new(max_iterations));
    }

    // Ensure max_iterations is set
    {
        let impl_state = state.implementation_state.as_mut().unwrap();
        if impl_state.max_iterations == 0 {
            impl_state.max_iterations = max_iterations;
        }
        // Start from iteration 1 if not started
        if impl_state.iteration == 0 {
            impl_state.iteration = 1;
        }
    }

    // Emit initial state update so TUI switches to implementation palette
    session_sender.send_state_update(state.clone());
    session_sender.send_phase_started("Implementing".to_string());

    // Use initial feedback if provided
    let mut current_feedback = initial_feedback.or_else(|| {
        state
            .implementation_state
            .as_ref()
            .and_then(|s| s.last_feedback.clone())
    });

    // Track last fingerprint for circuit breaker
    let mut last_fingerprint: Option<u64> = None;

    session_sender.send_output(format!(
        "[implementation] Starting implementation workflow (max {} iterations)",
        max_iterations
    ));

    // Main orchestration loop
    loop {
        // Check if we can continue
        let (iteration, can_continue) = {
            let impl_state = state.implementation_state.as_ref().unwrap();
            (impl_state.iteration, impl_state.can_continue())
        };

        if !can_continue {
            break;
        }

        // === Implementation Phase ===
        {
            let impl_state = state.implementation_state.as_mut().unwrap();
            impl_state.phase = ImplementationPhase::Implementing;
        }
        session_sender.send_output(format!(
            "[implementation] === Implementation Round {}/{} ===",
            iteration, max_iterations
        ));

        // Ensure agent_conversations entry exists for implementing agent (for conversation ID capture)
        if let Some(agent_cfg) = impl_config.implementing.as_ref() {
            let key = implementing_conversation_key(&agent_cfg.agent);
            state.get_or_create_agent_session(&key, ResumeStrategy::ConversationResume);
        }

        let impl_result = run_implementation_phase(
            state,
            config,
            working_dir,
            iteration,
            current_feedback.as_deref(),
            session_sender.clone(),
            session_logger.clone(),
        )
        .await
        .context("Implementation phase failed")?;

        // Check if implementation was cancelled
        if impl_result.stop_reason.as_deref() == Some("cancelled") {
            let impl_state = state.implementation_state.as_mut().unwrap();
            impl_state.phase = ImplementationPhase::Complete;
            return Ok(ImplementationWorkflowResult::Cancelled {
                iterations_used: iteration,
            });
        }

        // Check if implementation had an error
        if impl_result.is_error {
            session_sender.send_output(format!(
                "[implementation] Implementation error: {}",
                impl_result
                    .stop_reason
                    .as_deref()
                    .unwrap_or("unknown error")
            ));
            // Continue to review - let the review agent assess the state
        }

        // Update conversation ID if captured
        if let Some(conv_id) = impl_result.conversation_id {
            if let Some(agent_cfg) = impl_config.implementing.as_ref() {
                let key = implementing_conversation_key(&agent_cfg.agent);
                state.update_agent_conversation_id(&key, conv_id);
            }
        }

        // === Review Phase ===
        {
            let impl_state = state.implementation_state.as_mut().unwrap();
            impl_state.phase = ImplementationPhase::ImplementationReview;
        }
        // Emit state update and phase event for review phase
        session_sender.send_state_update(state.clone());
        session_sender.send_phase_started("Implementation Review".to_string());
        session_sender.send_output(format!(
            "[implementation] === Review Round {}/{} ===",
            iteration, max_iterations
        ));

        let review_result = run_implementation_review_phase(
            state,
            config,
            working_dir,
            iteration,
            Some(&impl_result.log_path),
            session_sender.clone(),
            session_logger.clone(),
        )
        .await
        .context("Implementation review phase failed")?;

        // Store the verdict
        {
            let impl_state = state.implementation_state.as_mut().unwrap();
            impl_state.last_verdict = Some(review_result.verdict.to_state_string());
        }

        // Handle verdict
        match review_result.verdict {
            VerificationVerdictResult::Approved => {
                let impl_state = state.implementation_state.as_mut().unwrap();
                impl_state.phase = ImplementationPhase::Complete;
                impl_state.mark_complete();
                // Emit state update so TUI reverts to planning palette
                session_sender.send_state_update(state.clone());
                session_sender.send_phase_started("Implementation Complete".to_string());
                session_sender.send_output("[implementation] Implementation approved!".to_string());
                // Emit success event to trigger the success modal in TUI
                session_sender.send_implementation_success(iteration);
                return Ok(ImplementationWorkflowResult::Approved);
            }
            VerificationVerdictResult::NeedsRevision
            | VerificationVerdictResult::ParseFailure { .. } => {
                // Store feedback for next iteration
                {
                    let impl_state = state.implementation_state.as_mut().unwrap();
                    impl_state.last_feedback = review_result.feedback.clone();
                }
                current_feedback = review_result.feedback;

                // Circuit breaker: check if anything changed
                let current_fingerprint = compute_change_fingerprint(working_dir).unwrap_or(0);

                if let Some(prev_fp) = last_fingerprint {
                    if prev_fp == current_fingerprint {
                        session_sender.send_output(
                            "[implementation] No changes detected between iterations, stopping"
                                .to_string(),
                        );
                        let impl_state = state.implementation_state.as_mut().unwrap();
                        impl_state.phase = ImplementationPhase::Complete;
                        return Ok(ImplementationWorkflowResult::NoChanges {
                            iterations_used: iteration,
                        });
                    }
                }
                last_fingerprint = Some(current_fingerprint);

                // Check if we have more iterations
                if iteration >= max_iterations {
                    session_sender.send_output(format!(
                        "[implementation] Max iterations ({}) reached without approval",
                        max_iterations
                    ));
                    let impl_state = state.implementation_state.as_mut().unwrap();
                    impl_state.phase = ImplementationPhase::Complete;
                    return Ok(ImplementationWorkflowResult::Failed {
                        iterations_used: iteration,
                        last_feedback: current_feedback,
                    });
                }

                // Advance to next iteration
                {
                    let impl_state = state.implementation_state.as_mut().unwrap();
                    impl_state.advance_to_next_iteration();
                }
                let new_iteration = state.implementation_state.as_ref().unwrap().iteration;
                // Emit state update for new iteration
                session_sender.send_state_update(state.clone());
                session_sender.send_phase_started("Implementing".to_string());
                session_sender.send_output(format!(
                    "[implementation] Issues found, starting iteration {}...",
                    new_iteration
                ));
            }
        }
    }

    // Should not reach here, but handle gracefully
    let iteration = state
        .implementation_state
        .as_ref()
        .map(|s| s.iteration)
        .unwrap_or(0);
    Ok(ImplementationWorkflowResult::Failed {
        iterations_used: iteration,
        last_feedback: current_feedback,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implementation_workflow_result_variants() {
        // Just verify the enum variants compile
        let _approved = ImplementationWorkflowResult::Approved;
        let _failed = ImplementationWorkflowResult::Failed {
            iterations_used: 3,
            last_feedback: Some("Fix bugs".to_string()),
        };
        let _cancelled = ImplementationWorkflowResult::Cancelled { iterations_used: 1 };
        let _no_changes = ImplementationWorkflowResult::NoChanges { iterations_used: 2 };
    }
}
