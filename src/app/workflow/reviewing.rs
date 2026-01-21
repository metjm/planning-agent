//! Reviewing phase execution.

use super::WorkflowResult;
use crate::app::util::{build_all_reviewers_failed_summary, build_review_failure_summary};
use crate::app::workflow_decisions::{
    handle_max_iterations, wait_for_all_reviewers_failed_decision, wait_for_review_decision,
    AllReviewersFailedDecision, ReviewDecision,
};
use crate::config::{AgentRef, WorkflowConfig};
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    write_feedback_files,
};
use crate::session_logger::{LogCategory, LogLevel, SessionLogger};
use crate::state::{FeedbackStatus, Phase, State};
use crate::tui::{CancellationError, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct WorkflowPhaseContext<'a> {
    pub working_dir: &'a Path,
    pub state_path: &'a Path,
    pub config: &'a WorkflowConfig,
    pub sender: &'a SessionEventSender,
    /// Session logger for workflow events.
    pub session_logger: Arc<SessionLogger>,
}

impl<'a> WorkflowPhaseContext<'a> {
    /// Logs a workflow message to the session logger.
    pub fn log_workflow(&self, message: &str) {
        self.session_logger.log(LogLevel::Info, LogCategory::Workflow, message);
    }

    /// Logs a workflow message at a specific level.
    #[allow(dead_code)]
    pub fn log_workflow_level(&self, level: LogLevel, message: &str) {
        self.session_logger.log(level, LogCategory::Workflow, message);
    }
}

pub async fn run_reviewing_phase(
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
    context.log_workflow(&format!(
        ">>> ENTERING Reviewing phase (iteration {})",
        state.iteration
    ));
    sender.send_phase_started("Reviewing".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!(
        "=== REVIEW PHASE (Iteration {}) ===",
        state.iteration
    ));
    let reviewer_display_names: Vec<&str> = config
        .workflow
        .reviewing
        .agents
        .iter()
        .map(|r| r.display_id())
        .collect();
    sender.send_output(format!("Reviewers: {}", reviewer_display_names.join(", ")));

    let mut reviews_by_agent: HashMap<String, phases::ReviewResult> = HashMap::new();
    let mut pending_reviewers: Vec<AgentRef> = config.workflow.reviewing.agents.clone();
    let mut retry_attempts = 0usize;

    loop {
        // Check for commands before running reviewers
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    context.log_workflow( &format!("Received interrupt during reviewing: {}", feedback));
                    sender.send_output("[review] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart { user_feedback: feedback }));
                }
                WorkflowCommand::Stop => {
                    context.log_workflow( "Received stop during reviewing");
                    sender.send_output("[review] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        let pending_display_ids: Vec<&str> =
            pending_reviewers.iter().map(|r| r.display_id()).collect();
        context.log_workflow(&format!("Running reviewers: {:?}", pending_display_ids));
        let batch = run_multi_agent_review_with_context(
            state,
            working_dir,
            config,
            &pending_reviewers,
            sender.clone(),
            state.iteration,
            state_path,
            context.session_logger.clone(),
        )
        .await;

        // Check for cancellation
        let batch = match batch {
            Ok(b) => b,
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                context.log_workflow( "Review phase was cancelled");
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

        // Collect failed display_ids
        let failed_display_ids: Vec<String> = batch
            .failures
            .iter()
            .map(|f| f.agent_name.clone())
            .collect();

        // Find the AgentRef objects that match failed display_ids
        let failed_agent_refs: Vec<AgentRef> = config
            .workflow
            .reviewing
            .agents
            .iter()
            .filter(|r| failed_display_ids.contains(&r.display_id().to_string()))
            .cloned()
            .collect();

        if reviews_by_agent.is_empty() {
            let max_retries = config.failure_policy.max_retries as usize;
            if retry_attempts < max_retries {
                retry_attempts += 1;
                sender.send_output(format!(
                    "[review] All reviewers failed; retrying ({}/{})...",
                    retry_attempts, max_retries
                ));
                pending_reviewers = failed_agent_refs.clone();
                continue;
            }

            // Output bundle paths before prompting for decision
            output_failure_bundles(sender, &batch.failures);

            // Prompt user for recovery decision instead of hard-bailing
            sender.send_output("[review] All reviewers failed after retries; awaiting your decision...".to_string());
            let summary = build_all_reviewers_failed_summary(
                &batch.failures,
                retry_attempts,
                max_retries,
            );
            sender.send_all_reviewers_failed(summary);

            let decision = wait_for_all_reviewers_failed_decision(working_dir, approval_rx, control_rx).await;

            match decision {
                AllReviewersFailedDecision::Retry => {
                    context.log_workflow( "User chose to retry all reviewers");
                    retry_attempts = 0; // Reset retry counter for fresh attempt
                    pending_reviewers = failed_agent_refs.clone();
                    continue;
                }
                AllReviewersFailedDecision::Stop => {
                    context.log_workflow( "User chose to stop and save state");
                    return Ok(Some(WorkflowResult::Stopped));
                }
                AllReviewersFailedDecision::Abort => {
                    context.log_workflow( "User chose to abort after all reviewers failed");
                    return Ok(Some(WorkflowResult::Aborted {
                        reason: "All reviewers failed - user chose to abort".to_string(),
                    }));
                }
                AllReviewersFailedDecision::Stopped => {
                    context.log_workflow( "Workflow stopped during all reviewers failed decision");
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        sender.send_output("[review] Some reviewers failed; awaiting your decision...".to_string());
        let summary = build_review_failure_summary(&reviews_by_agent, &batch.failures);
        sender.send_review_decision_request(summary);

        let decision = wait_for_review_decision(working_dir, approval_rx, control_rx).await;

        match decision {
            ReviewDecision::Retry => {
                pending_reviewers = failed_agent_refs;
                continue;
            }
            ReviewDecision::Continue => {
                break;
            }
            ReviewDecision::Stopped => {
                context.log_workflow( "Workflow stopped during review decision");
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
    context.log_workflow( &format!("Aggregated status: {:?}", status));

    // Signal round completion for review history UI
    let round_approved = matches!(status, FeedbackStatus::Approved);
    sender.send_review_round_completed(state.iteration, round_approved);

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
        context.session_logger.clone(),
    );

    match status {
        FeedbackStatus::Approved => {
            context.log_workflow( "Plan APPROVED! Transitioning to Complete");
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
                context.log_workflow( "Transitioning: Reviewing -> Revising");
                state.transition(Phase::Revising)?;
            }
        }
    }
    state.set_updated_at();
    state.save_atomic(state_path)?;
    sender.send_state_update(state.clone());

    Ok(None)
}

/// Outputs diagnostics bundle paths for failures that have them.
fn output_failure_bundles(sender: &SessionEventSender, failures: &[phases::ReviewFailure]) {
    let mut has_bundles = false;
    for failure in failures {
        if let Some(ref path) = failure.bundle_path {
            sender.send_output(format!(
                "[diagnostics] {}: {}",
                failure.agent_name,
                path.display()
            ));
            has_bundles = true;
        }
    }
    if has_bundles {
        sender.send_output("[warning] Bundles may contain sensitive information from logs.".to_string());
    }
}
