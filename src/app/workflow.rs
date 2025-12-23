
use crate::app::util::{build_max_iterations_summary, build_review_failure_summary, log_workflow};
use crate::app::workflow_common::{cleanup_merged_feedback, REVIEW_FAILURE_RETRY_LIMIT};
use crate::config::WorkflowConfig;
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    run_planning_phase_with_context, run_revision_phase_with_context, write_feedback_files,
};
use crate::state::{FeedbackStatus, Phase, State};
use crate::tui::{Event, SessionEventSender, UserApprovalResponse};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub enum WorkflowResult {

    Accepted,

    NeedsRestart { user_feedback: String },

    Aborted { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ReviewDecision {
    Retry,
    Continue,
}

pub async fn run_workflow_with_config(
    mut state: State,
    working_dir: PathBuf,
    state_path: PathBuf,
    config: WorkflowConfig,
    output_tx: mpsc::UnboundedSender<Event>,
    mut approval_rx: mpsc::Receiver<UserApprovalResponse>,
    session_id: usize,
) -> Result<WorkflowResult> {
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
        &format!("Workflow session ID: {}", state.workflow_session_id),
    );

    let sender = SessionEventSender::new(session_id, output_tx.clone());
    let mut last_reviews: Vec<phases::ReviewResult> = Vec::new();

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                run_planning_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                )
                .await?;
            }

            Phase::Reviewing => {
                let result = run_reviewing_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut last_reviews,
                )
                .await?;

                if let Some(workflow_result) = result {
                    return Ok(workflow_result);
                }

                if state.phase == Phase::Complete {
                    break;
                }
            }

            Phase::Revising => {
                run_revising_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut last_reviews,
                )
                .await?;
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
    working_dir: &PathBuf,
    state_path: &PathBuf,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
) -> Result<()> {
    log_workflow(working_dir, ">>> ENTERING Planning phase");
    sender.send_phase_started("Planning".to_string());
    sender.send_output("".to_string());
    sender.send_output("=== PLANNING PHASE ===".to_string());
    sender.send_output(format!("Feature: {}", state.feature_name));
    sender.send_output(format!("Agent: {}", config.workflow.planning.agent));
    sender.send_output(format!("Plan file: {}", state.plan_file.display()));

    log_workflow(working_dir, "Calling run_planning_phase_with_context...");
    run_planning_phase_with_context(state, working_dir, config, sender.clone(), state_path).await?;
    log_workflow(working_dir, "run_planning_phase_with_context completed");

    let plan_path = working_dir.join(&state.plan_file);
    if !plan_path.exists() {
        log_workflow(working_dir, "ERROR: Plan file was not created!");
        sender.send_output("[error] Plan file was not created!".to_string());
        anyhow::bail!("Plan file not created");
    }

    log_workflow(working_dir, "Transitioning: Planning -> Reviewing");
    state.transition(Phase::Reviewing)?;
    state.save(state_path)?;
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

    Ok(())
}

async fn run_reviewing_phase(
    state: &mut State,
    working_dir: &PathBuf,
    state_path: &PathBuf,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<Option<WorkflowResult>> {
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
        .await?;

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

        let decision = wait_for_review_decision(working_dir, approval_rx).await;

        match decision {
            ReviewDecision::Retry => {
                pending_reviewers = failed_names;
                continue;
            }
            ReviewDecision::Continue => {
                break;
            }
        }
    }

    let mut reviews: Vec<phases::ReviewResult> = reviews_by_agent.into_values().collect();
    reviews.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));

    let feedback_path = working_dir.join(&state.feedback_file);
    let _ = write_feedback_files(&reviews, &feedback_path);
    let _ = merge_feedback(&reviews, &feedback_path);

    let status = aggregate_reviews(&reviews, &config.workflow.reviewing.aggregation);
    log_workflow(working_dir, &format!("Aggregated status: {:?}", status));

    *last_reviews = reviews;
    state.last_feedback_status = Some(status.clone());

    let review_phase_name = if state.iteration > 1 {
        format!("Reviewing #{}", state.iteration)
    } else {
        "Reviewing".to_string()
    };
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
    state.save(state_path)?;
    sender.send_state_update(state.clone());

    Ok(None)
}

async fn wait_for_review_decision(
    working_dir: &PathBuf,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
) -> ReviewDecision {
    match approval_rx.recv().await {
        Some(UserApprovalResponse::ReviewRetry) => ReviewDecision::Retry,
        Some(UserApprovalResponse::ReviewContinue) => ReviewDecision::Continue,
        Some(UserApprovalResponse::Accept) => {
            log_workflow(
                working_dir,
                "Received plan approval while awaiting review decision, treating as continue",
            );
            ReviewDecision::Continue
        }
        Some(UserApprovalResponse::Decline(_)) => {
            log_workflow(
                working_dir,
                "Received plan decline while awaiting review decision, treating as retry",
            );
            ReviewDecision::Retry
        }
        Some(UserApprovalResponse::PlanGenerationRetry) => {
            log_workflow(
                working_dir,
                "Received PlanGenerationRetry while awaiting review decision, treating as retry",
            );
            ReviewDecision::Retry
        }
        Some(UserApprovalResponse::AbortWorkflow) => {
            log_workflow(
                working_dir,
                "Received AbortWorkflow while awaiting review decision, treating as continue",
            );
            ReviewDecision::Continue
        }
        Some(UserApprovalResponse::ProceedWithoutApproval) => {
            log_workflow(
                working_dir,
                "Received ProceedWithoutApproval while awaiting review decision, treating as continue",
            );
            ReviewDecision::Continue
        }
        Some(UserApprovalResponse::ContinueReviewing) => {
            log_workflow(
                working_dir,
                "Received ContinueReviewing while awaiting review decision, treating as continue",
            );
            ReviewDecision::Continue
        }
        None => {
            log_workflow(
                working_dir,
                "Review decision channel closed, treating as continue",
            );
            ReviewDecision::Continue
        }
    }
}

async fn handle_max_iterations(
    state: &mut State,
    working_dir: &PathBuf,
    state_path: &PathBuf,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    last_reviews: &[phases::ReviewResult],
) -> Result<Option<WorkflowResult>> {
    log_workflow(working_dir, "Max iterations reached - prompting user");
    sender.send_output("[planning] Max iterations reached".to_string());
    sender.send_output("[planning] Awaiting your decision...".to_string());

    let summary = build_max_iterations_summary(state, working_dir, last_reviews);
    sender.send_max_iterations_reached(summary);

    loop {
        match approval_rx.recv().await {
            Some(UserApprovalResponse::ProceedWithoutApproval) => {
                log_workflow(working_dir, "User chose to proceed without AI approval");
                sender.send_output("[planning] Proceeding without AI approval...".to_string());
                state.approval_overridden = true;
                state.transition(Phase::Complete)?;
                state.save(state_path)?;
                return Ok(None);
            }
            Some(UserApprovalResponse::ContinueReviewing) => {
                log_workflow(working_dir, "User chose to continue reviewing");
                sender.send_output("[planning] Continuing with another review cycle...".to_string());
                state.max_iterations += 1;
                state.transition(Phase::Revising)?;
                state.save(state_path)?;
                return Ok(None);
            }
            Some(UserApprovalResponse::Decline(feedback)) => {
                log_workflow(
                    working_dir,
                    &format!("User declined with feedback: {}", feedback),
                );
                sender.send_output(format!("[planning] Restarting with feedback: {}", feedback));
                return Ok(Some(WorkflowResult::NeedsRestart {
                    user_feedback: feedback,
                }));
            }
            Some(UserApprovalResponse::AbortWorkflow) => {
                log_workflow(working_dir, "User chose to abort workflow");
                sender.send_output("[planning] Workflow aborted by user".to_string());
                return Ok(Some(WorkflowResult::Aborted {
                    reason: "User aborted workflow at max iterations".to_string(),
                }));
            }
            Some(other) => {
                log_workflow(
                    working_dir,
                    &format!(
                        "Ignoring unexpected response {:?} during max iterations prompt",
                        other
                    ),
                );
                continue;
            }
            None => {
                log_workflow(working_dir, "Approval channel closed - aborting");
                return Ok(Some(WorkflowResult::Aborted {
                    reason: "Approval channel closed".to_string(),
                }));
            }
        }
    }
}

async fn run_revising_phase(
    state: &mut State,
    working_dir: &PathBuf,
    state_path: &PathBuf,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<()> {
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
    run_revision_phase_with_context(
        state,
        working_dir,
        config,
        last_reviews,
        sender.clone(),
        state.iteration,
        state_path,
    )
    .await?;
    last_reviews.clear();
    log_workflow(working_dir, "run_revision_phase_with_context completed");

    let feedback_path = working_dir.join(&state.feedback_file);
    match cleanup_merged_feedback(&feedback_path) {
        Ok(true) => log_workflow(working_dir, "Deleted old feedback file"),
        Ok(false) => {}
        Err(e) => log_workflow(
            working_dir,
            &format!("Warning: Failed to delete feedback file: {}", e),
        ),
    }

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
    log_workflow(
        working_dir,
        &format!(
            "Transitioning: Revising -> Reviewing (iteration now {})",
            state.iteration
        ),
    );
    state.transition(Phase::Reviewing)?;
    state.save(state_path)?;
    sender.send_state_update(state.clone());
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    Ok(())
}

async fn handle_completion(
    state: &State,
    working_dir: &PathBuf,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
) -> Result<WorkflowResult> {
    log_workflow(working_dir, ">>> Plan complete - requesting user approval");

    sender.send_output("".to_string());

    let plan_path = working_dir.join(&state.plan_file);

    if state.approval_overridden {
        sender.send_output("=== PROCEEDING WITHOUT AI APPROVAL ===".to_string());
        sender.send_output("User chose to proceed after max iterations".to_string());
        sender.send_output("Waiting for your final decision...".to_string());

        let summary = format!(
            "You chose to proceed without AI approval after {} review iterations.\n\n\
             Plan file: {}\n\n\
             Available actions:\n\
             - **[i] Implement**: Launch Claude to implement the unapproved plan\n\
             - **[d] Decline**: Provide feedback and restart the workflow",
            state.iteration,
            plan_path.display()
        );
        sender.send_user_override_approval(summary);
    } else {
        sender.send_output("=== PLAN APPROVED BY AI ===".to_string());
        sender.send_output(format!("Completed after {} iteration(s)", state.iteration));
        sender.send_output("Waiting for your approval...".to_string());

        let summary = format!(
            "The plan has been approved by AI review.\n\nPlan file: {}",
            plan_path.display()
        );
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
