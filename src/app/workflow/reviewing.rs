//! Reviewing phase execution.

use super::WorkflowResult;
use crate::app::util::{build_all_reviewers_failed_summary, build_review_failure_summary};
use crate::app::workflow_decisions::{
    wait_for_all_reviewers_failed_decision, wait_for_review_decision, AllReviewersFailedDecision,
    ReviewDecision,
};
use crate::config::{AgentRef, WorkflowConfig};
use crate::domain::actor::WorkflowMessage;
use crate::domain::review::ReviewMode;
use crate::domain::review::SequentialReviewState;
use crate::domain::types::AgentId;
use crate::domain::types::{FeedbackPath, FeedbackStatus};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    write_feedback_files,
};
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{
    CancellationError, ReviewKind, SessionEventSender, UserApprovalResponse, WorkflowCommand,
};
use anyhow::anyhow;
use anyhow::Result;
use ractor::ActorRef;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

pub struct WorkflowPhaseContext<'a> {
    pub working_dir: &'a Path,
    pub config: &'a WorkflowConfig,
    pub sender: &'a SessionEventSender,
    /// Session logger for workflow events.
    pub session_logger: Arc<SessionLogger>,
    /// Actor reference for dispatching domain commands.
    pub actor_ref: Option<ActorRef<WorkflowMessage>>,
}

impl<'a> WorkflowPhaseContext<'a> {
    /// Logs a workflow message to the session logger.
    pub fn log_workflow(&self, message: &str) {
        self.session_logger
            .log(LogLevel::Info, LogCategory::Workflow, message);
    }

    /// Dispatches a domain command to the workflow actor.
    /// Returns Ok(()) if the command was sent successfully, or an error message if not.
    /// This is fire-and-forget for now - we don't wait for the result.
    pub async fn dispatch_command(&self, cmd: DomainCommand) {
        if let Some(ref actor) = self.actor_ref {
            let (reply_tx, reply_rx) = oneshot::channel();
            if let Err(e) =
                actor.send_message(WorkflowMessage::Command(Box::new(cmd.clone()), reply_tx))
            {
                self.log_workflow(&format!("Failed to dispatch command {:?}: {}", cmd, e));
                return;
            }
            // Wait for reply to ensure command was processed
            match reply_rx.await {
                Ok(Ok(_view)) => {
                    self.log_workflow(&format!("Command dispatched: {:?}", cmd));
                }
                Ok(Err(e)) => {
                    self.log_workflow(&format!("Command rejected: {:?}: {:?}", cmd, e));
                }
                Err(_) => {
                    self.log_workflow(&format!("Command reply channel dropped: {:?}", cmd));
                }
            }
        }
    }
}

pub async fn run_reviewing_phase(
    view: &WorkflowView,
    context: &WorkflowPhaseContext<'_>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<Option<WorkflowResult>> {
    let working_dir = context.working_dir;
    let config = context.config;
    let sender = context.sender;
    let iteration = view.iteration().unwrap_or_default().0;
    context.log_workflow(&format!(
        ">>> ENTERING Reviewing phase (iteration {})",
        iteration
    ));
    sender.send_phase_started("Reviewing".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!("=== REVIEW PHASE (Iteration {}) ===", iteration));
    let reviewer_display_names: Vec<&str> = config
        .workflow
        .reviewing
        .agents
        .iter()
        .map(|r| r.display_id())
        .collect();
    sender.send_output(format!("Reviewers: {}", reviewer_display_names.join(", ")));

    // Dispatch ReviewCycleStarted command to CQRS actor
    let reviewer_ids: Vec<AgentId> = reviewer_display_names
        .iter()
        .map(|s| AgentId::from(*s))
        .collect();
    context
        .dispatch_command(DomainCommand::ReviewCycleStarted {
            mode: ReviewMode::Parallel,
            reviewers: reviewer_ids,
        })
        .await;

    let mut reviews_by_agent: HashMap<String, phases::ReviewResult> = HashMap::new();
    let mut pending_reviewers: Vec<AgentRef> = config.workflow.reviewing.agents.clone();
    let mut retry_attempts = 0usize;

    loop {
        // Check for commands before running reviewers
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    context.log_workflow(&format!(
                        "Received interrupt during reviewing: {}",
                        feedback
                    ));
                    sender.send_output("[review] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart {
                        user_feedback: feedback,
                    }));
                }
                WorkflowCommand::Stop => {
                    context.log_workflow("Received stop during reviewing");
                    sender.send_output("[review] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        let pending_display_ids: Vec<&str> =
            pending_reviewers.iter().map(|r| r.display_id()).collect();
        context.log_workflow(&format!("Running reviewers: {:?}", pending_display_ids));
        let batch = run_multi_agent_review_with_context(
            view,
            working_dir,
            config,
            &pending_reviewers,
            sender.clone(),
            iteration,
            context.session_logger.clone(),
            true, // emit_round_started: parallel mode always emits
            context.actor_ref.clone(),
        )
        .await;

        // Check for cancellation
        let batch = match batch {
            Ok(b) => b,
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                context.log_workflow("Review phase was cancelled");
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
            let max_retries = config.failure_policy.max_retries() as usize;
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
            sender.send_output(
                "[review] All reviewers failed after retries; awaiting your decision..."
                    .to_string(),
            );
            let summary =
                build_all_reviewers_failed_summary(&batch.failures, retry_attempts, max_retries);
            sender.send_all_reviewers_failed(summary);

            let decision = wait_for_all_reviewers_failed_decision(
                &context.session_logger,
                approval_rx,
                control_rx,
            )
            .await;

            match decision {
                AllReviewersFailedDecision::Retry => {
                    context.log_workflow("User chose to retry all reviewers");
                    retry_attempts = 0; // Reset retry counter for fresh attempt
                    pending_reviewers = failed_agent_refs.clone();
                    continue;
                }
                AllReviewersFailedDecision::Stop => {
                    context.log_workflow("User chose to stop and save state");
                    return Ok(Some(WorkflowResult::Stopped));
                }
                AllReviewersFailedDecision::Abort => {
                    context.log_workflow("User chose to abort after all reviewers failed");
                    // Dispatch UserAborted command
                    let reason = "All reviewers failed - user chose to abort".to_string();
                    context
                        .dispatch_command(DomainCommand::UserAborted {
                            reason: reason.clone(),
                        })
                        .await;
                    return Ok(Some(WorkflowResult::Aborted { reason }));
                }
                AllReviewersFailedDecision::Stopped => {
                    context.log_workflow("Workflow stopped during all reviewers failed decision");
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        sender.send_output("[review] Some reviewers failed; awaiting your decision...".to_string());
        let summary = build_review_failure_summary(&reviews_by_agent, &batch.failures);
        sender.send_review_decision_request(summary);

        let decision =
            wait_for_review_decision(&context.session_logger, approval_rx, control_rx).await;

        match decision {
            ReviewDecision::Retry => {
                pending_reviewers = failed_agent_refs;
                continue;
            }
            ReviewDecision::Continue => {
                break;
            }
            ReviewDecision::Stopped => {
                context.log_workflow("Workflow stopped during review decision");
                return Ok(Some(WorkflowResult::Stopped));
            }
        }
    }

    let mut reviews: Vec<phases::ReviewResult> = reviews_by_agent.into_values().collect();
    reviews.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));

    // Get feedback path from view (already an absolute path)
    let feedback_path = view
        .feedback_path()
        .map(|fp| fp.0.clone())
        .unwrap_or_else(|| std::path::PathBuf::from("feedback.md"));
    if let Err(e) = write_feedback_files(&reviews, &feedback_path) {
        context.session_logger.log(
            LogLevel::Warn,
            LogCategory::Workflow,
            &format!("Failed to write feedback files: {}", e),
        );
    }
    if let Err(e) = merge_feedback(&reviews, &feedback_path) {
        context.session_logger.log(
            LogLevel::Warn,
            LogCategory::Workflow,
            &format!("Failed to merge feedback: {}", e),
        );
    }

    let status = aggregate_reviews(&reviews, &config.workflow.reviewing.aggregation);
    context.log_workflow(&format!("Aggregated status: {:?}", status));

    // Dispatch ReviewerApproved/ReviewerRejected for each reviewer
    for review in &reviews {
        let reviewer_id = AgentId::from(review.agent_name.as_str());
        if review.needs_revision {
            context
                .dispatch_command(DomainCommand::ReviewerRejected {
                    reviewer_id,
                    feedback_path: FeedbackPath::from(feedback_path.clone()),
                })
                .await;
        } else {
            context
                .dispatch_command(DomainCommand::ReviewerApproved { reviewer_id })
                .await;
        }
    }

    // Signal round completion for review history UI
    let round_approved = matches!(status, FeedbackStatus::Approved);
    sender.send_review_round_completed(ReviewKind::Plan, iteration, round_approved);

    // Dispatch ReviewCycleCompleted command to CQRS actor
    context
        .dispatch_command(DomainCommand::ReviewCycleCompleted {
            approved: round_approved,
        })
        .await;

    *last_reviews = reviews;

    let review_phase_name = format!("Reviewing #{}", iteration);
    phases::spawn_summary_generation(
        review_phase_name,
        view,
        working_dir,
        config,
        sender.clone(),
        Some(last_reviews),
        context.session_logger.clone(),
    );

    let max_iterations = view.max_iterations().map(|m| m.0).unwrap_or(3);
    match status {
        FeedbackStatus::Approved => {
            context.log_workflow("Plan APPROVED! Transitioning to Complete");
            sender.send_output("[planning] Plan APPROVED!".to_string());
            // Transition is handled by ReviewCycleCompleted event
        }
        FeedbackStatus::NeedsRevision => {
            sender.send_output("[planning] Plan needs revision".to_string());
            if iteration >= max_iterations {
                // Dispatch PlanningMaxIterationsReached and return - main loop handles AwaitingPlanningDecision
                context
                    .dispatch_command(DomainCommand::PlanningMaxIterationsReached)
                    .await;
                // Return to let the main workflow loop handle the AwaitingPlanningDecision phase
                return Ok(None);
            } else {
                context.log_workflow("Transitioning: Reviewing -> Revising");
                // Transition is handled by ReviewCycleCompleted event
            }
        }
    }

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
        sender.send_output(
            "[warning] Bundles may contain sensitive information from logs.".to_string(),
        );
    }
}

/// Build max iterations summary using WorkflowView's plan_path.
pub(crate) fn build_max_iterations_summary_from_view(
    view: &WorkflowView,
    last_reviews: &[phases::ReviewResult],
) -> String {
    use crate::app::util::truncate_for_summary;

    let plan_path = view
        .plan_path()
        .map(|p| p.0.display().to_string())
        .unwrap_or_else(|| "plan.md".to_string());
    let iteration = view.iteration().unwrap_or_default().0;

    let mut summary = format!(
        "The plan has been reviewed {} times but has not been approved by AI.\n\nPlan file: {}\n\n",
        iteration, plan_path
    );

    if let Some(ref status) = view.last_feedback_status() {
        summary.push_str(&format!("Last review verdict: {:?}\n\n", status));
    }

    if !last_reviews.is_empty() {
        // Review Summary with verdict grouping
        summary.push_str("---\n\n## Review Summary\n\n");

        let needs_revision_count = last_reviews.iter().filter(|r| r.needs_revision).count();
        let approved_count = last_reviews.len() - needs_revision_count;

        summary.push_str(&format!(
            "**{} reviewer(s):** {} needs revision, {} approved\n\n",
            last_reviews.len(),
            needs_revision_count,
            approved_count
        ));

        let needs_revision: Vec<_> = last_reviews.iter().filter(|r| r.needs_revision).collect();
        let approved: Vec<_> = last_reviews.iter().filter(|r| !r.needs_revision).collect();

        if !needs_revision.is_empty() {
            let names: Vec<_> = needs_revision
                .iter()
                .map(|r| r.agent_name.to_uppercase())
                .collect();
            summary.push_str(&format!("**Needs Revision:** {}\n\n", names.join(", ")));
        }

        if !approved.is_empty() {
            let names: Vec<_> = approved
                .iter()
                .map(|r| r.agent_name.to_uppercase())
                .collect();
            summary.push_str(&format!("**Approved:** {}\n\n", names.join(", ")));
        }

        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            let truncated_summary = truncate_for_summary(&review.summary, 120);
            summary.push_str(&format!(
                "- **{}** - **{}**: {}\n",
                review.agent_name.to_uppercase(),
                verdict,
                truncated_summary
            ));
        }
        summary.push('\n');

        // Preview section
        summary.push_str("---\n\n## Latest Review Feedback (Preview)\n\n");
        summary.push_str("_Scroll down for full feedback_\n\n");
        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            summary.push_str(&format!(
                "### {} ({})\n\n",
                review.agent_name.to_uppercase(),
                verdict
            ));
            let preview: String = review
                .feedback
                .lines()
                .take(5)
                .collect::<Vec<_>>()
                .join("\n");
            summary.push_str(&format!("{}\n\n", truncate_for_summary(&preview, 300)));
        }

        // Full feedback section
        summary.push_str("---\n\n## Full Review Feedback\n\n");
        for review in last_reviews {
            let verdict = if review.needs_revision {
                "NEEDS REVISION"
            } else {
                "APPROVED"
            };
            summary.push_str(&format!(
                "### {} ({})\n\n",
                review.agent_name.to_uppercase(),
                verdict
            ));
            summary.push_str(&format!("{}\n\n", review.feedback));
        }
    } else {
        summary.push_str("---\n\n_No review feedback available._\n\n");
    }

    summary.push_str("---\n\n");
    summary.push_str("Choose an action:\n");
    summary.push_str("- **[p] Proceed**: Accept the current plan and continue to implementation\n");
    summary.push_str(
        "- **[c] Continue Review**: Run another review cycle (adds 1 to max iterations)\n",
    );
    summary.push_str(
        "- **[d] Restart with Feedback**: Provide feedback to restart the entire workflow\n",
    );

    summary
}

/// Sequential reviewing phase: runs reviewers one at a time.
///
/// # Architecture
/// This function follows the CQRS pattern:
/// 1. Read state from view (read-only)
/// 2. Dispatch ONE command
/// 3. Return immediately (let main loop re-read view and call again)
///
/// The aggregate handles all state transitions via events.
///
/// # Flow
/// - If no review_mode or needs_cycle_start: dispatch ReviewCycleStarted, return
/// - If all reviewers processed (index >= len): dispatch ReviewCycleCompleted, return
/// - Otherwise: run current reviewer, dispatch result, return
pub async fn run_sequential_reviewing_phase(
    view: &WorkflowView,
    context: &WorkflowPhaseContext<'_>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<Option<WorkflowResult>> {
    let working_dir = context.working_dir;
    let config = context.config;
    let sender = context.sender;
    let reviewers = &config.workflow.reviewing.agents;
    let iteration = view.iteration().unwrap_or_default().0;
    let reviewer_ids: Vec<&str> = reviewers.iter().map(|r| r.display_id()).collect();

    // =========================================================================
    // STEP 1: Initialize cycle if needed
    // =========================================================================
    let seq_state = match view.review_mode() {
        Some(ReviewMode::Sequential(state)) => state,
        _ => {
            // No sequential state yet - initialize via ReviewCycleStarted
            context.log_workflow("Initializing sequential review cycle");
            let reviewer_agent_ids: Vec<AgentId> =
                reviewer_ids.iter().map(|s| AgentId::from(*s)).collect();

            context
                .dispatch_command(DomainCommand::ReviewCycleStarted {
                    mode: ReviewMode::Sequential(SequentialReviewState::new_with_cycle(
                        &reviewer_ids,
                    )),
                    reviewers: reviewer_agent_ids,
                })
                .await;

            // Return - main loop will re-read view with initialized state
            return Ok(None);
        }
    };

    // Check if cycle order needs initialization (empty after revision)
    if seq_state.needs_cycle_start() {
        context.log_workflow("Re-initializing sequential review cycle after revision");
        let reviewer_agent_ids: Vec<AgentId> =
            reviewer_ids.iter().map(|s| AgentId::from(*s)).collect();

        context
            .dispatch_command(DomainCommand::ReviewCycleStarted {
                mode: ReviewMode::Sequential(SequentialReviewState::new_with_cycle(&reviewer_ids)),
                reviewers: reviewer_agent_ids,
            })
            .await;

        // Return - main loop will re-read view
        return Ok(None);
    }

    // =========================================================================
    // STEP 2: Check if all reviewers have been processed
    // =========================================================================
    let current_index = seq_state.current_reviewer_index();
    let plan_version = seq_state.plan_version();

    // If index >= len, we've run all reviewers - check results and complete
    if seq_state.get_current_reviewer().is_none() {
        context.log_workflow("All reviewers have been processed, checking results");

        // Check if all approved the current version
        let all_approved = seq_state.all_approved(&reviewer_ids);

        if all_approved {
            context.log_workflow("All reviewers approved - plan complete!");
            sender.send_output("[sequential] All reviewers approved - plan complete!".to_string());
            sender.send_review_round_completed(ReviewKind::Plan, iteration, true);

            // Write merged feedback with accumulated reviews
            let feedback_path = view
                .feedback_path()
                .map(|fp| fp.0.clone())
                .unwrap_or_else(|| std::path::PathBuf::from("feedback.md"));
            let all_reviews = seq_state.get_accumulated_reviews_for_summary();
            if let Err(e) = merge_feedback(&all_reviews, &feedback_path) {
                context.session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Failed to merge feedback: {}", e),
                );
            }

            // Spawn summary generation
            phases::spawn_summary_generation(
                format!("Reviewing #{}", iteration),
                view,
                working_dir,
                config,
                sender.clone(),
                Some(&all_reviews),
                context.session_logger.clone(),
            );

            context
                .dispatch_command(DomainCommand::ReviewCycleCompleted { approved: true })
                .await;
        } else {
            // This shouldn't happen in normal flow - if someone rejected, we would have
            // dispatched ReviewCycleCompleted(false) immediately. But handle it gracefully.
            context.log_workflow("Not all reviewers approved (unexpected state)");
            sender.send_review_round_completed(ReviewKind::Plan, iteration, false);
            context
                .dispatch_command(DomainCommand::ReviewCycleCompleted { approved: false })
                .await;
        }

        return Ok(None);
    }

    // =========================================================================
    // STEP 3: Run the current reviewer
    // =========================================================================
    let current_id = seq_state.get_current_reviewer().unwrap(); // Safe: checked above
    let Some(reviewer) = reviewers
        .iter()
        .find(|r| r.display_id() == current_id.as_str())
    else {
        return Err(anyhow!(
            "reviewer {} not found in config",
            current_id.as_str()
        ));
    };
    let reviewer_id = reviewer.display_id();

    context.log_workflow(&format!(
        ">>> Sequential Reviewing: iteration {}, reviewer {}/{} ({}), plan version {}",
        iteration,
        current_index + 1,
        reviewers.len(),
        reviewer_id,
        plan_version
    ));

    sender.send_phase_started("Reviewing".to_string());
    sender.send_output(format!(
        "=== SEQUENTIAL REVIEW (Iteration {}, Reviewer {}/{}) ===",
        iteration,
        current_index + 1,
        reviewers.len()
    ));

    // Emit round started only for first reviewer
    if current_index == 0 {
        sender.send_review_round_started(ReviewKind::Plan, iteration);
    }

    sender.send_output(format!(
        "Running reviewer: {} (plan version {})",
        reviewer_id, plan_version
    ));

    // Run the reviewer with retry loop
    let review = run_single_reviewer_with_retries(
        view,
        reviewer,
        reviewer_id,
        iteration,
        context,
        approval_rx,
        control_rx,
    )
    .await?;

    // Handle early exit from retry loop
    let review = match review {
        Some(r) => r,
        None => return Ok(Some(WorkflowResult::Stopped)), // User stopped during retries
    };

    // Store review for potential revision feedback
    last_reviews.clear();
    last_reviews.push(review.clone());

    // Write feedback file
    let feedback_path = view
        .feedback_path()
        .map(|fp| fp.0.clone())
        .unwrap_or_else(|| std::path::PathBuf::from("feedback.md"));
    if let Err(e) = write_feedback_files(std::slice::from_ref(&review), &feedback_path) {
        context.session_logger.log(
            LogLevel::Warn,
            LogCategory::Workflow,
            &format!("Failed to write feedback files: {}", e),
        );
    }

    // =========================================================================
    // STEP 4: Handle the review result
    // =========================================================================
    let max_iterations = view.max_iterations().map(|m| m.0).unwrap_or(3);

    if review.needs_revision {
        // Reviewer rejected
        context.log_workflow(&format!(
            "Reviewer {} REJECTED (plan version {})",
            reviewer_id, plan_version
        ));
        sender.send_output(format!(
            "[sequential] {} REJECTED - will revise and restart",
            reviewer_id
        ));

        // Dispatch rejection event
        context
            .dispatch_command(DomainCommand::ReviewerRejected {
                reviewer_id: AgentId::from(reviewer_id),
                feedback_path: FeedbackPath::from(feedback_path.clone()),
            })
            .await;

        sender.send_review_round_completed(ReviewKind::Plan, iteration, false);

        // Complete the cycle (rejected)
        context
            .dispatch_command(DomainCommand::ReviewCycleCompleted { approved: false })
            .await;

        // Check iteration limit
        if iteration >= max_iterations {
            // Dispatch PlanningMaxIterationsReached and return - main loop handles AwaitingPlanningDecision
            context
                .dispatch_command(DomainCommand::PlanningMaxIterationsReached)
                .await;
            return Ok(None);
        }
    } else {
        // Reviewer approved
        context.log_workflow(&format!(
            "Reviewer {} APPROVED (plan version {})",
            reviewer_id, plan_version
        ));
        sender.send_output(format!(
            "[sequential] {} APPROVED (version {})",
            reviewer_id, plan_version
        ));

        // Dispatch approval event - aggregate will advance index
        context
            .dispatch_command(DomainCommand::ReviewerApproved {
                reviewer_id: AgentId::from(reviewer_id),
            })
            .await;

        // Return immediately - main loop will re-read view with updated index
        // On next call, if index >= len, we'll detect completion in STEP 2
    }

    Ok(None)
}

/// Runs a single reviewer with retry logic for failures.
/// Returns Some(review) on success, None if user stopped.
async fn run_single_reviewer_with_retries(
    view: &WorkflowView,
    reviewer: &AgentRef,
    reviewer_id: &str,
    iteration: u32,
    context: &WorkflowPhaseContext<'_>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> Result<Option<phases::ReviewResult>> {
    let working_dir = context.working_dir;
    let config = context.config;
    let sender = context.sender;
    let mut retry_attempts = 0usize;

    loop {
        // Check for commands before running
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    context.log_workflow(&format!(
                        "Received interrupt during sequential reviewing: {}",
                        feedback
                    ));
                    sender.send_output("[review] Interrupted by user".to_string());
                    return Ok(None);
                }
                WorkflowCommand::Stop => {
                    context.log_workflow("Received stop during sequential reviewing");
                    sender.send_output("[review] Stopping...".to_string());
                    return Ok(None);
                }
            }
        }

        let batch = run_multi_agent_review_with_context(
            view,
            working_dir,
            config,
            std::slice::from_ref(reviewer),
            sender.clone(),
            iteration,
            context.session_logger.clone(),
            false, // Don't emit round_started, we handle it in caller
            context.actor_ref.clone(),
        )
        .await;

        let batch = match batch {
            Ok(b) => b,
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                context.log_workflow("Sequential review phase was cancelled");
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        // Success
        if let Some(review) = batch.reviews.into_iter().next() {
            return Ok(Some(review));
        }

        // Failure - retry or prompt user
        if let Some(failure) = batch.failures.into_iter().next() {
            let max_retries = config.failure_policy.max_retries() as usize;
            if retry_attempts < max_retries {
                retry_attempts += 1;
                sender.send_output(format!(
                    "[review:{}] Failed; retrying ({}/{})...",
                    reviewer_id, retry_attempts, max_retries
                ));
                continue;
            }

            if let Some(ref path) = failure.bundle_path {
                sender.send_output(format!(
                    "[diagnostics] {}: {}",
                    failure.agent_name,
                    path.display()
                ));
            }

            sender.send_output(format!(
                "[review:{}] Failed after retries; awaiting your decision...",
                reviewer_id
            ));
            let summary = build_all_reviewers_failed_summary(
                std::slice::from_ref(&failure),
                retry_attempts,
                max_retries,
            );
            sender.send_all_reviewers_failed(summary);

            let decision = wait_for_all_reviewers_failed_decision(
                &context.session_logger,
                approval_rx,
                control_rx,
            )
            .await;

            match decision {
                AllReviewersFailedDecision::Retry => {
                    context.log_workflow(&format!("User chose to retry reviewer {}", reviewer_id));
                    retry_attempts = 0;
                    continue;
                }
                AllReviewersFailedDecision::Stop => {
                    context.log_workflow("User chose to stop and save state");
                    return Ok(None);
                }
                AllReviewersFailedDecision::Abort => {
                    let reason = format!("Reviewer {} failed - user chose to abort", reviewer_id);
                    context
                        .dispatch_command(DomainCommand::UserAborted {
                            reason: reason.clone(),
                        })
                        .await;
                    return Ok(None);
                }
                AllReviewersFailedDecision::Stopped => {
                    return Ok(None);
                }
            }
        }

        anyhow::bail!("Review returned no results and no failures");
    }
}
