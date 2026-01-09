
use crate::app::util::{build_approval_summary, build_plan_failure_summary, build_review_failure_summary, log_workflow};
use crate::app::workflow_common::{plan_file_has_content, REVIEW_FAILURE_RETRY_LIMIT};
use crate::app::workflow_decisions::{
    handle_max_iterations, wait_for_plan_failure_decision, wait_for_review_decision,
    PlanFailureDecision, ReviewDecision,
};
use crate::config::WorkflowConfig;
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    run_planning_phase_with_context, run_revision_phase_with_context, write_feedback_files,
};
use crate::state::{FeedbackStatus, Phase, State};
use crate::tui::{CancellationError, Event, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub enum WorkflowResult {
    Accepted,
    NeedsRestart { user_feedback: String },
    Aborted { reason: String },
    /// Workflow was cleanly stopped at a phase boundary
    Stopped,
}

pub struct WorkflowRunConfig {
    pub working_dir: PathBuf,
    pub state_path: PathBuf,
    pub config: WorkflowConfig,
    pub output_tx: mpsc::UnboundedSender<Event>,
    pub approval_rx: mpsc::Receiver<UserApprovalResponse>,
    pub control_rx: mpsc::Receiver<WorkflowCommand>,
    pub session_id: usize,
    pub run_id: u64,
}

pub async fn run_workflow_with_config(
    mut state: State,
    run_config: WorkflowRunConfig,
) -> Result<WorkflowResult> {
    let WorkflowRunConfig {
        working_dir,
        state_path,
        config,
        output_tx,
        mut approval_rx,
        mut control_rx,
        session_id,
        run_id,
    } = run_config;
    log_workflow(
        &working_dir,
        &format!(
            "=== WORKFLOW START (multi-agent): {} ===",
            state.feature_name
        ),
    );
    log_workflow(
        &working_dir,
        &format!(
            "Config: planning={}, reviewers={:?}, revising={}",
            config.workflow.planning.agent,
            config.workflow.reviewing.agents,
            config.workflow.revising.agent
        ),
    );
    log_workflow(
        &working_dir,
        &format!("Workflow session ID: {}, run ID: {}", state.workflow_session_id, run_id),
    );

    let sender = SessionEventSender::new(session_id, run_id, output_tx.clone());
    let mut last_reviews: Vec<phases::ReviewResult> = Vec::new();
    let phase_context = WorkflowPhaseContext {
        working_dir: &working_dir,
        state_path: &state_path,
        config: &config,
        sender: &sender,
    };

    while state.should_continue() {
        // Check for commands before each phase
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    log_workflow(&working_dir, &format!("Received interrupt with feedback: {}", feedback));
                    sender.send_output("[workflow] Interrupted by user".to_string());
                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                }
                WorkflowCommand::Stop => {
                    log_workflow(&working_dir, "Received stop command at phase boundary");
                    sender.send_output("[workflow] Stopping at phase boundary...".to_string());
                    // Save snapshot is done by the TUI layer before returning
                    return Ok(WorkflowResult::Stopped);
                }
            }
        }

        match state.phase {
            Phase::Planning => {
                let result = run_planning_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => return Ok(workflow_result),
                    Ok(None) => {}
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        // Phase was cancelled - check for command
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Planning phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Planning phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Cancellation without command - shouldn't happen, but treat as abort
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Reviewing => {
                let result = run_reviewing_phase(
                    &mut state,
                    &phase_context,
                    &mut approval_rx,
                    &mut control_rx,
                    &mut last_reviews,
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => return Ok(workflow_result),
                    Ok(None) => {
                        if state.phase == Phase::Complete {
                            break;
                        }
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Reviewing phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Reviewing phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Revising => {
                let result = run_revising_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut control_rx,
                    &mut last_reviews,
                )
                .await;

                match result {
                    Ok(()) => {}
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Revising phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Revising phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Complete => {
                break;
            }
        }
    }

    log_workflow(
        &working_dir,
        &format!(
            "=== WORKFLOW END: phase={:?}, iteration={} ===",
            state.phase, state.iteration
        ),
    );

    if state.phase == Phase::Complete {
        return handle_completion(
            &state,
            &working_dir,
            &sender,
            &mut approval_rx,
        )
        .await;
    }

    sender.send_output("".to_string());
    sender.send_output("=== WORKFLOW COMPLETE ===".to_string());
    sender.send_output("Max iterations reached. Manual review recommended.".to_string());

    Ok(WorkflowResult::Accepted)
}

async fn run_planning_phase(
    state: &mut State,
    working_dir: &Path,
    state_path: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> Result<Option<WorkflowResult>> {
    log_workflow(working_dir, ">>> ENTERING Planning phase");
    sender.send_phase_started("Planning".to_string());
    sender.send_output("".to_string());
    sender.send_output("=== PLANNING PHASE ===".to_string());
    sender.send_output(format!("Feature: {}", state.feature_name));
    sender.send_output(format!("Agent: {}", config.workflow.planning.agent));
    sender.send_output(format!("Plan file: {}", state.plan_file.display()));

    // state.plan_file is now an absolute path (in ~/.planning-agent/plans/)
    let plan_path = state.plan_file.clone();

    loop {
        // Check for commands before starting planning
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    log_workflow(working_dir, &format!("Received interrupt during planning: {}", feedback));
                    sender.send_output("[planning] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart { user_feedback: feedback }));
                }
                WorkflowCommand::Stop => {
                    log_workflow(working_dir, "Received stop during planning");
                    sender.send_output("[planning] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        log_workflow(working_dir, "Calling run_planning_phase_with_context...");
        let planning_result =
            run_planning_phase_with_context(state, working_dir, config, sender.clone(), state_path)
                .await;

        match planning_result {
            Ok(()) => {
                log_workflow(working_dir, "run_planning_phase_with_context completed");

                // Use content-based check instead of exists() for pre-created files
                if !plan_file_has_content(&plan_path) {
                    log_workflow(working_dir, "ERROR: Plan file has no content!");
                    sender.send_output("[error] Plan file has no content - planning agent may have failed".to_string());

                    // Prompt user for decision
                    let summary = build_plan_failure_summary(
                        "Plan file has no content - planning agent may have failed",
                        &plan_path,
                        false,
                    );
                    sender.send_plan_generation_failed(summary);

                    match wait_for_plan_failure_decision(working_dir, approval_rx, control_rx, false).await {
                        PlanFailureDecision::Retry => {
                            sender.send_output("[planning] Retrying plan generation...".to_string());
                            continue;
                        }
                        PlanFailureDecision::Continue => {
                            // This shouldn't happen since plan has no content, but handle it
                            sender.send_output("[planning] Plan file has no content to continue with. Retrying...".to_string());
                            continue;
                        }
                        PlanFailureDecision::Abort => {
                            log_workflow(working_dir, "User aborted after plan file empty");
                            return Ok(Some(WorkflowResult::Aborted {
                                reason: "User aborted: plan file has no content".to_string(),
                            }));
                        }
                        PlanFailureDecision::Stopped => {
                            log_workflow(working_dir, "Workflow stopped during plan failure decision");
                            return Ok(Some(WorkflowResult::Stopped));
                        }
                    }
                }

                // Plan file has content and planning succeeded
                break;
            }
            Err(e) => {
                // Check if this is a cancellation error
                if e.downcast_ref::<CancellationError>().is_some() {
                    log_workflow(working_dir, "Planning phase was cancelled");
                    // Re-throw to be handled by caller
                    return Err(e);
                }

                let error_msg = format!("{}", e);
                log_workflow(
                    working_dir,
                    &format!("Planning phase error: {}", error_msg),
                );
                sender.send_output(format!("[error] Planning failed: {}", error_msg));

                // Use content-based check instead of exists() for pre-created files
                let plan_has_content = plan_file_has_content(&plan_path);
                let summary = build_plan_failure_summary(&error_msg, &plan_path, plan_has_content);
                sender.send_plan_generation_failed(summary);

                match wait_for_plan_failure_decision(working_dir, approval_rx, control_rx, plan_has_content).await {
                    PlanFailureDecision::Retry => {
                        sender.send_output("[planning] Retrying plan generation...".to_string());
                        continue;
                    }
                    PlanFailureDecision::Continue => {
                        if plan_has_content {
                            sender.send_output(
                                "[planning] Continuing with existing plan file...".to_string(),
                            );
                            break;
                        } else {
                            sender.send_output("[planning] Plan file has no content to continue with. Retrying...".to_string());
                            continue;
                        }
                    }
                    PlanFailureDecision::Abort => {
                        log_workflow(working_dir, "User aborted after planning error");
                        return Ok(Some(WorkflowResult::Aborted {
                            reason: format!("User aborted: {}", error_msg),
                        }));
                    }
                    PlanFailureDecision::Stopped => {
                        log_workflow(working_dir, "Workflow stopped during plan failure decision");
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                }
            }
        }
    }

    log_workflow(working_dir, "Transitioning: Planning -> Reviewing");
    state.transition(Phase::Reviewing)?;
    state.set_updated_at();
    state.save_atomic(state_path)?;
    sender.send_state_update(state.clone());
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    phases::spawn_summary_generation(
        "Planning".to_string(),
        state,
        working_dir,
        config,
        sender.clone(),
        None,
    );

    Ok(None)
}

struct WorkflowPhaseContext<'a> {
    working_dir: &'a Path,
    state_path: &'a Path,
    config: &'a WorkflowConfig,
    sender: &'a SessionEventSender,
}

async fn run_reviewing_phase(
    state: &mut State,
    context: &WorkflowPhaseContext<'_>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<Option<WorkflowResult>> {
    let working_dir = context.working_dir;
    let state_path = context.state_path;
    let config = context.config;
    let sender = context.sender;
    log_workflow(
        working_dir,
        &format!(
            ">>> ENTERING Reviewing phase (iteration {})",
            state.iteration
        ),
    );
    sender.send_phase_started("Reviewing".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!(
        "=== REVIEW PHASE (Iteration {}) ===",
        state.iteration
    ));
    sender.send_output(format!(
        "Reviewers: {}",
        config.workflow.reviewing.agents.join(", ")
    ));

    let mut reviews_by_agent: HashMap<String, phases::ReviewResult> = HashMap::new();
    let mut pending_reviewers = config.workflow.reviewing.agents.clone();
    let mut retry_attempts = 0usize;

    loop {
        // Check for commands before running reviewers
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    log_workflow(working_dir, &format!("Received interrupt during reviewing: {}", feedback));
                    sender.send_output("[review] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart { user_feedback: feedback }));
                }
                WorkflowCommand::Stop => {
                    log_workflow(working_dir, "Received stop during reviewing");
                    sender.send_output("[review] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        log_workflow(
            working_dir,
            &format!("Running reviewers: {:?}", pending_reviewers),
        );
        let batch = run_multi_agent_review_with_context(
            state,
            working_dir,
            config,
            &pending_reviewers,
            sender.clone(),
            state.iteration,
            state_path,
        )
        .await;

        // Check for cancellation
        let batch = match batch {
            Ok(b) => b,
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                log_workflow(working_dir, "Review phase was cancelled");
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        for review in batch.reviews {
            reviews_by_agent.insert(review.agent_name.clone(), review);
        }

        if batch.failures.is_empty() {
            break;
        }

        let failed_names = batch
            .failures
            .iter()
            .map(|f| f.agent_name.clone())
            .collect::<Vec<_>>();

        if reviews_by_agent.is_empty() {
            if retry_attempts < REVIEW_FAILURE_RETRY_LIMIT {
                retry_attempts += 1;
                sender.send_output(format!(
                    "[review] All reviewers failed; retrying ({}/{})...",
                    retry_attempts, REVIEW_FAILURE_RETRY_LIMIT
                ));
                pending_reviewers = failed_names;
                continue;
            }
            sender.send_output("[error] All reviewers failed; aborting review.".to_string());
            anyhow::bail!("All reviewers failed to complete review");
        }

        sender.send_output("[review] Some reviewers failed; awaiting your decision...".to_string());
        let summary = build_review_failure_summary(&reviews_by_agent, &batch.failures);
        sender.send_review_decision_request(summary);

        let decision = wait_for_review_decision(working_dir, approval_rx, control_rx).await;

        match decision {
            ReviewDecision::Retry => {
                pending_reviewers = failed_names;
                continue;
            }
            ReviewDecision::Continue => {
                break;
            }
            ReviewDecision::Stopped => {
                log_workflow(working_dir, "Workflow stopped during review decision");
                return Ok(Some(WorkflowResult::Stopped));
            }
        }
    }

    let mut reviews: Vec<phases::ReviewResult> = reviews_by_agent.into_values().collect();
    reviews.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));

    // state.feedback_file is now an absolute path (in ~/.planning-agent/plans/)
    let feedback_path = state.feedback_file.clone();
    let _ = write_feedback_files(&reviews, &feedback_path);
    let _ = merge_feedback(&reviews, &feedback_path);

    let status = aggregate_reviews(&reviews, &config.workflow.reviewing.aggregation);
    log_workflow(working_dir, &format!("Aggregated status: {:?}", status));

    *last_reviews = reviews;
    state.last_feedback_status = Some(status.clone());

    let review_phase_name = format!("Reviewing #{}", state.iteration);
    phases::spawn_summary_generation(
        review_phase_name,
        state,
        working_dir,
        config,
        sender.clone(),
        Some(last_reviews),
    );

    match status {
        FeedbackStatus::Approved => {
            log_workflow(working_dir, "Plan APPROVED! Transitioning to Complete");
            sender.send_output("[planning] Plan APPROVED!".to_string());
            state.transition(Phase::Complete)?;
        }
        FeedbackStatus::NeedsRevision => {
            sender.send_output("[planning] Plan needs revision".to_string());
            if state.iteration >= state.max_iterations {
                let result = handle_max_iterations(
                    state,
                    working_dir,
                    state_path,
                    sender,
                    approval_rx,
                    control_rx,
                    last_reviews,
                )
                .await?;
                if let Some(workflow_result) = result {
                    return Ok(Some(workflow_result));
                }
            } else {
                log_workflow(working_dir, "Transitioning: Reviewing -> Revising");
                state.transition(Phase::Revising)?;
            }
        }
    }
    state.set_updated_at();
    state.save_atomic(state_path)?;
    sender.send_state_update(state.clone());

    Ok(None)
}

async fn run_revising_phase(
    state: &mut State,
    working_dir: &Path,
    state_path: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<()> {
    // Check for commands before starting revision
    if let Ok(cmd) = control_rx.try_recv() {
        match cmd {
            WorkflowCommand::Interrupt { feedback } => {
                log_workflow(working_dir, &format!("Received interrupt during revising: {}", feedback));
                sender.send_output("[revision] Interrupted by user".to_string());
                return Err(CancellationError { feedback }.into());
            }
            WorkflowCommand::Stop => {
                log_workflow(working_dir, "Received stop during revising");
                sender.send_output("[revision] Stopping...".to_string());
                // Return a special error that signals stop - will be handled by caller
                return Err(anyhow::anyhow!("Workflow stopped"));
            }
        }
    }

    log_workflow(
        working_dir,
        &format!(
            ">>> ENTERING Revising phase (iteration {})",
            state.iteration
        ),
    );
    sender.send_phase_started("Revising".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!(
        "=== REVISION PHASE (Iteration {}) ===",
        state.iteration
    ));
    sender.send_output(format!("Agent: {}", config.workflow.revising.agent));

    log_workflow(working_dir, "Calling run_revision_phase_with_context...");
    let revision_result = run_revision_phase_with_context(
        state,
        working_dir,
        config,
        last_reviews,
        sender.clone(),
        state.iteration,
        state_path,
    )
    .await;

    // Check for cancellation
    match revision_result {
        Ok(()) => {}
        Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
            log_workflow(working_dir, "Revising phase was cancelled");
            return Err(e);
        }
        Err(e) => return Err(e),
    }
    last_reviews.clear();
    log_workflow(working_dir, "run_revision_phase_with_context completed");

    // Keep old feedback files - don't cleanup
    // let feedback_path = working_dir.join(&state.feedback_file);
    // match cleanup_merged_feedback(&feedback_path) { ... }

    let revision_phase_name = format!("Revising #{}", state.iteration);
    phases::spawn_summary_generation(
        revision_phase_name,
        state,
        working_dir,
        config,
        sender.clone(),
        None,
    );

    state.iteration += 1;
    // Update feedback filename for the new iteration before transitioning to review
    state.update_feedback_for_iteration(state.iteration);
    log_workflow(
        working_dir,
        &format!(
            "Transitioning: Revising -> Reviewing (iteration now {})",
            state.iteration
        ),
    );
    state.transition(Phase::Reviewing)?;
    state.set_updated_at();
    state.save_atomic(state_path)?;
    sender.send_state_update(state.clone());
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    Ok(())
}

async fn handle_completion(
    state: &State,
    working_dir: &Path,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
) -> Result<WorkflowResult> {
    log_workflow(working_dir, ">>> Plan complete - requesting user approval");

    sender.send_output("".to_string());

    // state.plan_file is now an absolute path (in ~/.planning-agent/plans/)
    let plan_path = state.plan_file.clone();

    if state.approval_overridden {
        sender.send_output("=== PROCEEDING WITHOUT AI APPROVAL ===".to_string());
        sender.send_output("User chose to proceed after max iterations".to_string());
        sender.send_output("Waiting for your final decision...".to_string());

        let summary = build_approval_summary(&plan_path, true, state.iteration);
        sender.send_user_override_approval(summary);
    } else {
        sender.send_output("=== PLAN APPROVED BY AI ===".to_string());
        sender.send_output(format!("Completed after {} iteration(s)", state.iteration));
        sender.send_output("Waiting for your approval...".to_string());

        let summary = build_approval_summary(&plan_path, false, state.iteration);
        sender.send_approval_request(summary);
    };

    log_workflow(working_dir, "Waiting for user approval response...");
    loop {
        match approval_rx.recv().await {
            Some(UserApprovalResponse::Accept) => {
                log_workflow(working_dir, "User ACCEPTED the plan");
                sender.send_output("[planning] User accepted the plan!".to_string());
                return Ok(WorkflowResult::Accepted);
            }
            Some(UserApprovalResponse::Decline(feedback)) => {
                log_workflow(
                    working_dir,
                    &format!("User DECLINED with feedback: {}", feedback),
                );
                sender.send_output(format!("[planning] User requested changes: {}", feedback));
                return Ok(WorkflowResult::NeedsRestart {
                    user_feedback: feedback,
                });
            }
            Some(UserApprovalResponse::ReviewRetry)
            | Some(UserApprovalResponse::ReviewContinue) => {
                log_workflow(
                    working_dir,
                    "Received review decision while awaiting plan approval, ignoring",
                );
                continue;
            }
            Some(UserApprovalResponse::PlanGenerationRetry) => {
                log_workflow(
                    working_dir,
                    "Received PlanGenerationRetry while awaiting plan approval, ignoring",
                );
                continue;
            }
            Some(UserApprovalResponse::PlanGenerationContinue) => {
                log_workflow(
                    working_dir,
                    "Received PlanGenerationContinue while awaiting plan approval, ignoring",
                );
                continue;
            }
            Some(UserApprovalResponse::AbortWorkflow) => {
                log_workflow(
                    working_dir,
                    "Received AbortWorkflow while awaiting plan approval, ignoring",
                );
                continue;
            }
            Some(UserApprovalResponse::ProceedWithoutApproval) => {
                log_workflow(
                    working_dir,
                    "Received ProceedWithoutApproval while awaiting plan approval, ignoring",
                );
                continue;
            }
            Some(UserApprovalResponse::ContinueReviewing) => {
                log_workflow(
                    working_dir,
                    "Received ContinueReviewing while awaiting plan approval, ignoring",
                );
                continue;
            }
            None => {
                log_workflow(working_dir, "Approval channel closed - treating as accept");
                return Ok(WorkflowResult::Accepted);
            }
        }
    }
}
