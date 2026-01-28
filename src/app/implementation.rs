//! Implementation orchestrator for the JSON-mode implementation workflow.
//!
//! This module provides the `run_implementation_workflow` function that manages
//! the implementation -> review loop until approval or max iterations.

use crate::app::compute_change_fingerprint;
use crate::app::workflow_decisions::{
    await_max_iterations_decision, IterativePhase, MaxIterationsDecision,
};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::{
    AwaitingDecisionReason, ConversationId, ImplementationPhase, ImplementationVerdict, Iteration,
    ResumeStrategy,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::implementation::run_implementation_phase;
use crate::phases::implementation_review::run_implementation_review_phase;
use crate::phases::implementing_conversation_key;
use crate::phases::verdict::VerificationVerdictResult;
use crate::session_daemon::session_tracking::{
    ImplementationStateUpdate, SessionTracker, TerminalStateUpdate,
};
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::{Context, Result};
use ractor::ActorRef;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Sends terminal state to tracker. Terminal states ("Failed", "Cancelled") are
/// protocol-only strings derived from ImplementationWorkflowResult, not part of
/// the ImplementationPhase enum which covers in-progress phases only.
async fn send_terminal_tracker_update(
    tracker: &SessionTracker,
    workflow_session_id: &str,
    terminal_status_str: &str, // Renamed from terminal_phase_str for clarity
    iteration: u32,
    max_iterations: u32,
) {
    let _ = tracker
        .update_terminal_state(TerminalStateUpdate {
            workflow_session_id: workflow_session_id.to_string(),
            phase: "Implementation".to_string(), // Phase is always "Implementation"
            terminal_status: terminal_status_str.to_string(),
            iteration,
            max_iterations,
        })
        .await;
}

/// Context for the implementation workflow containing channels and resources.
pub struct ImplementationContext<'a> {
    pub session_sender: SessionEventSender,
    pub session_logger: Arc<SessionLogger>,
    pub approval_rx: &'a mut mpsc::Receiver<UserApprovalResponse>,
    pub control_rx: &'a mut mpsc::Receiver<WorkflowCommand>,
    pub actor_ref: Option<ActorRef<WorkflowMessage>>,
    /// Session tracker for updating daemon with implementation phase state.
    pub tracker: Arc<SessionTracker>,
    /// Workflow session ID for tracker updates.
    pub workflow_session_id: String,
}

/// Result of the implementation workflow.
#[derive(Debug, Clone)]
pub enum ImplementationWorkflowResult {
    /// Implementation was approved
    Approved,
    /// Implementation accepted by user override (max iterations reached)
    ApprovedOverridden { iterations_used: u32 },
    /// Implementation failed after max iterations (user chose abort)
    Failed {
        iterations_used: u32,
        last_feedback: Option<String>,
    },
    /// Implementation was cancelled by user
    Cancelled { iterations_used: u32 },
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
/// * `view` - The current workflow view (read-only projection)
/// * `config` - The workflow configuration
/// * `working_dir` - The working directory for implementation
/// * `ctx` - Implementation context containing channels and paths
/// * `initial_feedback` - Optional initial feedback to start with
pub async fn run_implementation_workflow(
    view: &WorkflowView,
    config: &WorkflowConfig,
    working_dir: &Path,
    ctx: ImplementationContext<'_>,
    initial_feedback: Option<String>,
) -> Result<ImplementationWorkflowResult> {
    // Destructure context for easier access
    let ImplementationContext {
        session_sender,
        session_logger,
        approval_rx,
        control_rx,
        actor_ref,
        tracker,
        workflow_session_id,
    } = ctx;

    // Helper to dispatch implementation commands
    let dispatch_impl_cmd = |cmd: DomainCommand| {
        let actor = actor_ref.clone();
        let logger = session_logger.clone();
        async move {
            if let Some(ref actor) = actor {
                let (reply_tx, reply_rx) = oneshot::channel();
                if let Err(e) =
                    actor.send_message(WorkflowMessage::Command(Box::new(cmd.clone()), reply_tx))
                {
                    logger.log(
                        LogLevel::Warn,
                        LogCategory::Workflow,
                        &format!("Failed to dispatch implementation command: {}", e),
                    );
                    return;
                }
                match reply_rx.await {
                    Ok(Ok(_)) => {
                        logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            &format!("Implementation command dispatched: {:?}", cmd),
                        );
                    }
                    Ok(Err(e)) => {
                        logger.log(
                            LogLevel::Warn,
                            LogCategory::Workflow,
                            &format!("Implementation command rejected: {:?}", e),
                        );
                    }
                    Err(_) => {
                        logger.log(
                            LogLevel::Warn,
                            LogCategory::Workflow,
                            "Implementation command reply channel dropped",
                        );
                    }
                }
            }
        }
    };

    // Validate config
    let impl_config = &config.implementation;
    if !impl_config.enabled {
        anyhow::bail!("Implementation is disabled in config");
    }

    let config_max_iterations = impl_config.max_iterations;

    // Get initial state from view (should exist - UserRequestedImplementation was called earlier)
    let (mut local_iteration, mut local_max_iterations, initial_phase) =
        if let Some(impl_state) = view.implementation_state() {
            (
                impl_state.iteration().0,
                impl_state.max_iterations().0,
                impl_state.phase(),
            )
        } else {
            // Implementation state not yet initialized - use config defaults
            // This shouldn't happen in normal flow but provides a fallback
            (1, config_max_iterations, ImplementationPhase::Implementing)
        };

    // Ensure max_iterations is set from config if view has 0
    if local_max_iterations == 0 {
        local_max_iterations = config_max_iterations;
    }
    // Start from iteration 1 if not started
    if local_iteration == 0 {
        local_iteration = 1;
    }

    session_sender.send_phase_started("Implementing".to_string());

    // Update tracker with initial Implementing phase at workflow start
    let _ = tracker
        .update(
            &workflow_session_id,
            "Implementation".to_string(),
            1,
            ImplementationPhase::Implementing.status_label().to_string(),
            Some(ImplementationStateUpdate {
                phase: ImplementationPhase::Implementing,
                iteration: local_iteration,
                max_iterations: local_max_iterations,
            }),
        )
        .await;

    // Use initial feedback if provided
    let mut current_feedback = initial_feedback.or_else(|| {
        view.implementation_state()
            .and_then(|s| s.last_feedback().map(|f| f.to_string()))
    });

    // Track last fingerprint for circuit breaker
    let mut last_fingerprint: Option<u64> = None;

    session_sender.send_output(format!(
        "[implementation] Starting implementation workflow (max {} iterations)",
        local_max_iterations
    ));

    // Track conversation ID across rounds
    let mut captured_conversation_id: Option<ConversationId> = None;

    // Main orchestration loop
    let mut local_phase = initial_phase;
    loop {
        // Check if we need to resume from awaiting decision
        if local_phase == ImplementationPhase::AwaitingDecision {
            // When resuming from AwaitingDecision, update tracker to restore implementation phase to host UI
            let _ = tracker
                .update(
                    &workflow_session_id,
                    "Implementation".to_string(),
                    1,
                    ImplementationPhase::AwaitingDecision
                        .status_label()
                        .to_string(),
                    Some(ImplementationStateUpdate {
                        phase: ImplementationPhase::AwaitingDecision,
                        iteration: local_iteration,
                        max_iterations: local_max_iterations,
                    }),
                )
                .await;

            // Determine the reason for being in AwaitingDecision phase
            let decision_reason = view
                .implementation_state()
                .and_then(|s| s.decision_reason());

            // Build appropriate summary based on the reason
            let summary = match decision_reason {
                Some(AwaitingDecisionReason::NoChanges) => {
                    // Use the iteration from view since we're resuming from event store
                    let iteration = view
                        .implementation_state()
                        .map(|s| s.iteration().0)
                        .unwrap_or(1);
                    build_implementation_no_changes_summary(current_feedback.as_deref(), iteration)
                }
                Some(AwaitingDecisionReason::MaxIterationsReached) | None => {
                    // Default to max iterations summary (backwards compatible for old sessions)
                    build_implementation_max_iterations_summary(view, current_feedback.as_deref())
                }
            };

            let decision = await_max_iterations_decision(
                IterativePhase::Implementation,
                &session_logger,
                &session_sender,
                approval_rx,
                control_rx,
                summary,
            )
            .await?;

            // Apply the decision using the shared helper
            if let Some(result) = apply_implementation_decision(
                decision,
                &dispatch_impl_cmd,
                &session_sender,
                &mut ImplementationLoopState {
                    iteration: &mut local_iteration,
                    max_iterations: &mut local_max_iterations,
                    phase: &mut local_phase,
                    feedback: &mut current_feedback,
                },
                &tracker,
                &workflow_session_id,
            )
            .await?
            {
                return Ok(result);
            }
            // If None returned, continue the loop (Continue or RestartWithFeedback)
        }

        // Check if we can continue
        let can_continue =
            local_phase != ImplementationPhase::Complete && local_iteration <= local_max_iterations;

        if !can_continue {
            break;
        }

        // === Implementation Phase ===
        session_sender.send_output(format!(
            "[implementation] === Implementation Round {}/{} ===",
            local_iteration, local_max_iterations
        ));

        // Dispatch ImplementationRoundStarted command
        dispatch_impl_cmd(DomainCommand::ImplementationRoundStarted {
            iteration: Iteration(local_iteration),
        })
        .await;

        // Update tracker with Implementing phase
        let _ = tracker
            .update(
                &workflow_session_id,
                "Implementation".to_string(),
                1,
                ImplementationPhase::Implementing.status_label().to_string(),
                Some(ImplementationStateUpdate {
                    phase: ImplementationPhase::Implementing,
                    iteration: local_iteration,
                    max_iterations: local_max_iterations,
                }),
            )
            .await;

        // NOTE: Pre-round RecordAgentConversation{None} was removed to preserve
        // conversation IDs across rounds and session resume

        let impl_result = run_implementation_phase(
            view,
            config,
            working_dir,
            local_iteration,
            current_feedback.as_deref(),
            captured_conversation_id.clone(),
            session_sender.clone(),
            session_logger.clone(),
            actor_ref.clone(),
        )
        .await
        .context("Implementation phase failed")?;

        // Check if implementation was cancelled
        if impl_result.stop_reason.as_deref() == Some("cancelled") {
            // Dispatch ImplementationCancelled command
            dispatch_impl_cmd(DomainCommand::ImplementationCancelled {
                reason: "Implementation cancelled by user".to_string(),
            })
            .await;

            // Update tracker with Cancelled terminal state (protocol-only string)
            send_terminal_tracker_update(
                &tracker,
                &workflow_session_id,
                "Cancelled",
                local_iteration,
                local_max_iterations,
            )
            .await;

            return Ok(ImplementationWorkflowResult::Cancelled {
                iterations_used: local_iteration,
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

        // Capture conversation ID for next round (using strong type)
        if let Some(ref conv_id) = impl_result.conversation_id {
            captured_conversation_id = Some(ConversationId::from(conv_id.clone()));
        }

        // Record agent conversation to event store (for session resume)
        if let Some(ref conv_id) = impl_result.conversation_id {
            if let Some(agent_cfg) = impl_config.implementing.as_ref() {
                let key = implementing_conversation_key(&agent_cfg.agent);
                dispatch_impl_cmd(DomainCommand::RecordAgentConversation {
                    agent_id: key.into(),
                    resume_strategy: ResumeStrategy::ConversationResume,
                    conversation_id: Some(conv_id.clone().into()),
                })
                .await;
            }
        }

        // Dispatch ImplementationRoundCompleted command
        let fingerprint = compute_change_fingerprint(working_dir).unwrap_or(0);
        dispatch_impl_cmd(DomainCommand::ImplementationRoundCompleted {
            iteration: Iteration(local_iteration),
            fingerprint,
        })
        .await;

        // Update tracker to show ImplementationReview phase
        let _ = tracker
            .update(
                &workflow_session_id,
                "Implementation".to_string(),
                1,
                ImplementationPhase::ImplementationReview
                    .status_label()
                    .to_string(),
                Some(ImplementationStateUpdate {
                    phase: ImplementationPhase::ImplementationReview,
                    iteration: local_iteration,
                    max_iterations: local_max_iterations,
                }),
            )
            .await;

        // === Review Phase ===
        session_sender.send_phase_started("Implementation Review".to_string());
        session_sender.send_output(format!(
            "[implementation] === Review Round {}/{} ===",
            local_iteration, local_max_iterations
        ));

        let review_result = run_implementation_review_phase(
            view,
            config,
            working_dir,
            local_iteration,
            Some(&impl_result.log_path),
            session_sender.clone(),
            session_logger.clone(),
        )
        .await
        .context("Implementation review phase failed")?;

        // Dispatch ImplementationReviewCompleted command (verdict stored via event)
        let domain_verdict = match &review_result.verdict {
            VerificationVerdictResult::Approved => ImplementationVerdict::Approved,
            // NeedsRevision and ParseFailure both map to NeedsChanges
            VerificationVerdictResult::NeedsRevision
            | VerificationVerdictResult::ParseFailure { .. } => ImplementationVerdict::NeedsChanges,
        };
        dispatch_impl_cmd(DomainCommand::ImplementationReviewCompleted {
            iteration: Iteration(local_iteration),
            verdict: domain_verdict,
            feedback: review_result.feedback.clone(),
        })
        .await;

        // Handle verdict
        match review_result.verdict {
            VerificationVerdictResult::Approved => {
                // Dispatch ImplementationAccepted command
                dispatch_impl_cmd(DomainCommand::ImplementationAccepted).await;

                // Update tracker to show Implementation Complete
                let _ = tracker
                    .update(
                        &workflow_session_id,
                        "Implementation".to_string(),
                        1,
                        ImplementationPhase::Complete.status_label().to_string(),
                        Some(ImplementationStateUpdate {
                            phase: ImplementationPhase::Complete,
                            iteration: local_iteration,
                            max_iterations: local_max_iterations,
                        }),
                    )
                    .await;

                session_sender.send_phase_started("Implementation Complete".to_string());
                session_sender.send_output("[implementation] Implementation approved!".to_string());
                // Emit success event to trigger the success modal in TUI
                session_sender.send_implementation_success(local_iteration);
                return Ok(ImplementationWorkflowResult::Approved);
            }
            VerificationVerdictResult::NeedsRevision
            | VerificationVerdictResult::ParseFailure { .. } => {
                // Store feedback for next iteration (via local tracking)
                current_feedback = review_result.feedback;

                // Circuit breaker: check if anything changed
                let current_fingerprint = compute_change_fingerprint(working_dir).unwrap_or(0);

                if let Some(prev_fp) = last_fingerprint {
                    if prev_fp == current_fingerprint {
                        session_sender.send_output(
                            "[implementation] No changes detected between iterations".to_string(),
                        );

                        // Dispatch ImplementationNoChanges command
                        dispatch_impl_cmd(DomainCommand::ImplementationNoChanges {
                            iteration: Iteration(local_iteration),
                        })
                        .await;

                        // Update tracker to show AwaitingDecision phase (no changes detected)
                        let _ = tracker
                            .update(
                                &workflow_session_id,
                                "Implementation".to_string(),
                                1,
                                ImplementationPhase::AwaitingDecision
                                    .status_label()
                                    .to_string(),
                                Some(ImplementationStateUpdate {
                                    phase: ImplementationPhase::AwaitingDecision,
                                    iteration: local_iteration,
                                    max_iterations: local_max_iterations,
                                }),
                            )
                            .await;

                        // Transition to awaiting decision state
                        local_phase = ImplementationPhase::AwaitingDecision;

                        // Build summary and prompt user
                        let summary = build_implementation_no_changes_summary(
                            current_feedback.as_deref(),
                            local_iteration,
                        );

                        let decision = await_max_iterations_decision(
                            IterativePhase::Implementation,
                            &session_logger,
                            &session_sender,
                            approval_rx,
                            control_rx,
                            summary,
                        )
                        .await?;

                        // Apply the decision (reuse existing function)
                        if let Some(result) = apply_implementation_decision(
                            decision,
                            &dispatch_impl_cmd,
                            &session_sender,
                            &mut ImplementationLoopState {
                                iteration: &mut local_iteration,
                                max_iterations: &mut local_max_iterations,
                                phase: &mut local_phase,
                                feedback: &mut current_feedback,
                            },
                            &tracker,
                            &workflow_session_id,
                        )
                        .await?
                        {
                            return Ok(result);
                        }
                        // If None returned, continue the loop
                        continue;
                    }
                }
                last_fingerprint = Some(current_fingerprint);

                // Check if we have more iterations
                if local_iteration >= local_max_iterations {
                    // Dispatch ImplementationMaxIterationsReached command
                    dispatch_impl_cmd(DomainCommand::ImplementationMaxIterationsReached).await;

                    // Update tracker to show AwaitingDecision phase (max iterations reached)
                    let _ = tracker
                        .update(
                            &workflow_session_id,
                            "Implementation".to_string(),
                            1,
                            ImplementationPhase::AwaitingDecision
                                .status_label()
                                .to_string(),
                            Some(ImplementationStateUpdate {
                                phase: ImplementationPhase::AwaitingDecision,
                                iteration: local_iteration,
                                max_iterations: local_max_iterations,
                            }),
                        )
                        .await;

                    // Transition to awaiting decision state
                    local_phase = ImplementationPhase::AwaitingDecision;

                    // Build summary and prompt user
                    let summary = build_implementation_max_iterations_summary(
                        view,
                        current_feedback.as_deref(),
                    );

                    let decision = await_max_iterations_decision(
                        IterativePhase::Implementation,
                        &session_logger,
                        &session_sender,
                        approval_rx,
                        control_rx,
                        summary,
                    )
                    .await?;

                    // Apply the decision
                    if let Some(result) = apply_implementation_decision(
                        decision,
                        &dispatch_impl_cmd,
                        &session_sender,
                        &mut ImplementationLoopState {
                            iteration: &mut local_iteration,
                            max_iterations: &mut local_max_iterations,
                            phase: &mut local_phase,
                            feedback: &mut current_feedback,
                        },
                        &tracker,
                        &workflow_session_id,
                    )
                    .await?
                    {
                        return Ok(result);
                    }
                    // If None returned, we continue (Continue or RestartWithFeedback)
                    // The loop will re-evaluate can_continue() on next iteration
                    continue;
                }

                // Advance to next iteration
                local_iteration += 1;

                // Update tracker for next implementation round
                let _ = tracker
                    .update(
                        &workflow_session_id,
                        "Implementation".to_string(),
                        1,
                        ImplementationPhase::Implementing.status_label().to_string(),
                        Some(ImplementationStateUpdate {
                            phase: ImplementationPhase::Implementing,
                            iteration: local_iteration,
                            max_iterations: local_max_iterations,
                        }),
                    )
                    .await;

                session_sender.send_phase_started("Implementing".to_string());
                session_sender.send_output(format!(
                    "[implementation] Issues found, starting iteration {}...",
                    local_iteration
                ));
            }
        }
    }

    // Should not reach here, but handle gracefully
    Ok(ImplementationWorkflowResult::Failed {
        iterations_used: local_iteration,
        last_feedback: current_feedback,
    })
}

/// Builds the summary text for implementation max iterations modal.
fn build_implementation_max_iterations_summary(
    view: &WorkflowView,
    last_feedback: Option<&str>,
) -> String {
    let impl_state = view.implementation_state();
    let iteration = impl_state.map(|s| s.iteration().0).unwrap_or(0);
    let max = impl_state.map(|s| s.max_iterations().0).unwrap_or(0);

    let mut summary = format!(
        "Implementation has been attempted {} time(s) (max: {}) but review has not approved.\n\n",
        iteration, max
    );

    if let Some(feedback) = last_feedback {
        summary.push_str("## Last Review Feedback\n\n");
        let preview = if feedback.chars().count() > 500 {
            let truncated: String = feedback.chars().take(500).collect();
            format!("{}...\n\n_(truncated)_", truncated)
        } else {
            feedback.to_string()
        };
        summary.push_str(&preview);
        summary.push_str("\n\n");
    }

    summary.push_str("---\n\n");
    summary.push_str("Choose an action:\n");
    summary.push_str("- **[y] Yes**: Accept current implementation without further review\n");
    summary.push_str("- **[c] Continue**: Run another implementation+review cycle\n");
    summary
        .push_str("- **[d] Decline**: Provide feedback to guide the next implementation attempt\n");
    summary.push_str("- **[a] Abort**: Stop the implementation workflow\n");

    summary
}

/// Builds the summary text for implementation no-changes modal.
/// Note: Unlike build_implementation_max_iterations_summary, this function takes iteration
/// directly because local_iteration is already available at the call site.
fn build_implementation_no_changes_summary(last_feedback: Option<&str>, iteration: u32) -> String {
    let mut summary = format!(
        "No changes were detected after implementation round {}.\n\n\
         This usually means the implementation agent either:\n\
         - Believes the implementation is complete\n\
         - Couldn't understand the feedback\n\
         - Encountered an issue it couldn't resolve\n\n",
        iteration
    );

    if let Some(feedback) = last_feedback {
        summary.push_str("## Last Review Feedback\n\n");
        let preview = if feedback.chars().count() > 500 {
            let truncated: String = feedback.chars().take(500).collect();
            format!("{}...\n\n_(truncated)_", truncated)
        } else {
            feedback.to_string()
        };
        summary.push_str(&preview);
        summary.push_str("\n\n");
    }

    summary.push_str("---\n\n");
    summary.push_str("Choose an action:\n");
    summary.push_str("- **[y] Yes**: Accept current implementation as-is\n");
    summary.push_str("- **[c] Continue**: Try another implementation round\n");
    summary.push_str("- **[d] Decline**: Provide different feedback to guide implementation\n");
    summary.push_str("- **[a] Abort**: Stop the implementation workflow\n");

    summary
}

/// Mutable loop state for the implementation workflow.
struct ImplementationLoopState<'a> {
    iteration: &'a mut u32,
    max_iterations: &'a mut u32,
    phase: &'a mut ImplementationPhase,
    feedback: &'a mut Option<String>,
}

/// Applies an implementation max iterations decision.
/// Used by both the main max iterations check and the AwaitingDecision resume handler.
///
/// This function dispatches domain commands and updates local loop state.
///
/// Returns:
/// - Some(result) if the decision completes the implementation workflow
/// - None if the loop should continue (e.g., Continue decision)
async fn apply_implementation_decision<F, Fut>(
    decision: MaxIterationsDecision,
    dispatch_impl_cmd: &F,
    session_sender: &SessionEventSender,
    loop_state: &mut ImplementationLoopState<'_>,
    tracker: &SessionTracker,
    workflow_session_id: &str,
) -> Result<Option<ImplementationWorkflowResult>>
where
    F: Fn(DomainCommand) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let iteration = *loop_state.iteration;
    let max_iterations = *loop_state.max_iterations;

    match decision {
        MaxIterationsDecision::ProceedWithoutApproval => {
            // Dispatch ImplementationAccepted command (user override)
            dispatch_impl_cmd(DomainCommand::ImplementationAccepted).await;

            // Update tracker with terminal state before returning
            send_terminal_tracker_update(
                tracker,
                workflow_session_id,
                "Complete",
                iteration,
                max_iterations,
            )
            .await;

            *loop_state.phase = ImplementationPhase::Complete;
            session_sender
                .send_output("[implementation] Proceeding without review approval".to_string());
            session_sender.send_implementation_success(iteration);
            Ok(Some(ImplementationWorkflowResult::ApprovedOverridden {
                iterations_used: iteration,
            }))
        }
        MaxIterationsDecision::Continue(additional) => {
            *loop_state.max_iterations += additional;
            *loop_state.phase = ImplementationPhase::Implementing;
            session_sender.send_output(format!(
                "[implementation] Continuing (max iterations now {})",
                *loop_state.max_iterations
            ));
            Ok(None) // Continue the loop
        }
        MaxIterationsDecision::RestartWithFeedback(feedback) => {
            *loop_state.iteration = 1;
            *loop_state.phase = ImplementationPhase::Implementing;
            // NOTE: Conversation ID is PRESERVED (not cleared)
            *loop_state.feedback = Some(feedback);
            session_sender
                .send_output("[implementation] Restarting with new feedback...".to_string());
            Ok(None) // Continue the loop from restart
        }
        MaxIterationsDecision::Abort => {
            dispatch_impl_cmd(DomainCommand::ImplementationDeclined {
                reason: "User declined at max iterations".to_string(),
            })
            .await;

            // Update tracker with terminal state before returning
            send_terminal_tracker_update(
                tracker,
                workflow_session_id,
                "Failed",
                iteration,
                max_iterations,
            )
            .await;

            *loop_state.phase = ImplementationPhase::Complete;
            Ok(Some(ImplementationWorkflowResult::Failed {
                iterations_used: iteration,
                last_feedback: loop_state.feedback.clone(),
            }))
        }
        MaxIterationsDecision::Stopped => {
            dispatch_impl_cmd(DomainCommand::ImplementationCancelled {
                reason: "User stopped implementation workflow".to_string(),
            })
            .await;

            // Update tracker with terminal state before returning
            send_terminal_tracker_update(
                tracker,
                workflow_session_id,
                "Cancelled",
                iteration,
                max_iterations,
            )
            .await;

            Ok(Some(ImplementationWorkflowResult::Cancelled {
                iterations_used: iteration,
            }))
        }
    }
}
