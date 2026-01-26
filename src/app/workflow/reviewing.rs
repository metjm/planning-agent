//! Reviewing phase execution.

use super::WorkflowResult;
use crate::app::util::{build_all_reviewers_failed_summary, build_review_failure_summary};
use crate::app::workflow_decisions::{
    await_max_iterations_decision, wait_for_all_reviewers_failed_decision,
    wait_for_review_decision, AllReviewersFailedDecision, IterativePhase, MaxIterationsDecision,
    ReviewDecision,
};
use crate::config::{AgentRef, WorkflowConfig};
use crate::domain::actor::WorkflowMessage;
use crate::domain::review::ReviewMode;
use crate::domain::review::{SequentialReviewState, SerializableReviewResult};
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
    let iteration = view.iteration.unwrap_or_default().0;
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
        .feedback_path
        .clone()
        .map(|fp| fp.0)
        .unwrap_or_else(|| std::path::PathBuf::from("feedback.md"));
    let _ = write_feedback_files(&reviews, &feedback_path);
    let _ = merge_feedback(&reviews, &feedback_path);

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

    let max_iterations = view.max_iterations.map(|m| m.0).unwrap_or(3);
    match status {
        FeedbackStatus::Approved => {
            context.log_workflow("Plan APPROVED! Transitioning to Complete");
            sender.send_output("[planning] Plan APPROVED!".to_string());
            // Transition is handled by ReviewCycleCompleted event
        }
        FeedbackStatus::NeedsRevision => {
            sender.send_output("[planning] Plan needs revision".to_string());
            if iteration >= max_iterations {
                // Dispatch PlanningMaxIterationsReached command
                context
                    .dispatch_command(DomainCommand::PlanningMaxIterationsReached)
                    .await;

                let result = handle_max_iterations_with_view(
                    view,
                    &context.session_logger,
                    sender,
                    approval_rx,
                    control_rx,
                    last_reviews,
                    context,
                )
                .await?;
                if let Some(workflow_result) = result {
                    return Ok(Some(workflow_result));
                }
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

/// Handles max iterations reached during reviewing phase using WorkflowView.
/// Dispatches appropriate commands based on user decision.
async fn handle_max_iterations_with_view(
    view: &WorkflowView,
    session_logger: &Arc<SessionLogger>,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &[phases::ReviewResult],
    context: &WorkflowPhaseContext<'_>,
) -> Result<Option<WorkflowResult>> {
    // Build the summary for planning phase
    let summary = build_max_iterations_summary_from_view(view, last_reviews);

    // Await user decision using the shared function
    let decision = await_max_iterations_decision(
        IterativePhase::Planning,
        session_logger,
        sender,
        approval_rx,
        control_rx,
        summary,
    )
    .await?;

    // Dispatch domain commands for user decisions
    match &decision {
        MaxIterationsDecision::ProceedWithoutApproval => {
            context
                .dispatch_command(DomainCommand::UserOverrideApproval {
                    override_reason: "User proceeded without AI approval at max iterations"
                        .to_string(),
                })
                .await;
            sender.send_output("[planning] Proceeding without AI approval...".to_string());
            Ok(None)
        }
        MaxIterationsDecision::Continue => {
            sender.send_output("[planning] Continuing with another review cycle...".to_string());
            // The caller should handle incrementing max_iterations via command
            Ok(None)
        }
        MaxIterationsDecision::RestartWithFeedback(feedback) => {
            sender.send_output(format!("[planning] Restarting with feedback: {}", feedback));
            Ok(Some(WorkflowResult::NeedsRestart {
                user_feedback: feedback.clone(),
            }))
        }
        MaxIterationsDecision::Abort => {
            context
                .dispatch_command(DomainCommand::UserAborted {
                    reason: "User aborted workflow at max iterations".to_string(),
                })
                .await;
            sender.send_output("[planning] Workflow aborted by user".to_string());
            Ok(Some(WorkflowResult::Aborted {
                reason: "User aborted workflow at max iterations".to_string(),
            }))
        }
        MaxIterationsDecision::Stopped => Ok(Some(WorkflowResult::Stopped)),
    }
}

/// Build max iterations summary using WorkflowView's plan_path.
pub(crate) fn build_max_iterations_summary_from_view(
    view: &WorkflowView,
    last_reviews: &[phases::ReviewResult],
) -> String {
    use crate::app::util::truncate_for_summary;

    let plan_path = view
        .plan_path
        .as_ref()
        .map(|p| p.0.display().to_string())
        .unwrap_or_else(|| "plan.md".to_string());
    let iteration = view.iteration.unwrap_or_default().0;

    let mut summary = format!(
        "The plan has been reviewed {} times but has not been approved by AI.\n\nPlan file: {}\n\n",
        iteration, plan_path
    );

    if let Some(ref status) = view.last_feedback_status {
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
/// If any reviewer rejects, immediately transitions to revision.
/// After revision, restarts from the first reviewer to ensure
/// all reviewers approve the same plan version.
///
/// Preserves all failure handling from parallel mode:
/// - Retry logic for transient failures
/// - User decision prompts for persistent failures
/// - Diagnostics bundle creation
///
/// TUI events: Emits `send_review_round_started` once at start of sequential cycle
/// (when first reviewer starts), then passes emit_round_started=false to
/// run_multi_agent_review_with_view to prevent duplicates.
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
    let iteration = view.iteration.unwrap_or_default().0;

    // Get sequential review state from view's review_mode
    let seq_state = match &view.review_mode {
        Some(ReviewMode::Sequential(state)) => state.clone(),
        _ => SequentialReviewState::new(),
    };

    // Validate reviewer state in case config changed between sessions
    let reviewer_ids: Vec<&str> = reviewers.iter().map(|r| r.display_id()).collect();
    let mut seq_state = seq_state;
    if seq_state.validate_reviewer_state(&reviewer_ids) {
        context.log_workflow("Sequential review: config changed, reset to start new cycle");
        sender.send_output(
            "[sequential] Reviewer configuration changed - restarting cycle".to_string(),
        );
    }

    // Dispatch ReviewCycleStarted command to CQRS actor (sequential mode)
    let reviewer_agent_ids: Vec<AgentId> = reviewer_ids.iter().map(|s| AgentId::from(*s)).collect();
    context
        .dispatch_command(DomainCommand::ReviewCycleStarted {
            mode: ReviewMode::Sequential(seq_state.clone()),
            reviewers: reviewer_agent_ids,
        })
        .await;

    context.log_workflow(&format!(
        ">>> ENTERING Sequential Reviewing phase (iteration {}, reviewer {}/{}, plan version {})",
        iteration,
        seq_state.current_reviewer_index + 1,
        reviewers.len(),
        seq_state.plan_version
    ));

    sender.send_phase_started("Reviewing".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!(
        "=== SEQUENTIAL REVIEW (Iteration {}, Reviewer {}/{}) ===",
        iteration,
        seq_state.current_reviewer_index + 1,
        reviewers.len()
    ));

    // Start new cycle if needed (cycle order empty after reset or config change)
    if seq_state.needs_cycle_start() {
        let tiebreaker = seq_state.start_new_cycle(&reviewer_ids);
        let mut log_msg = format!(
            "Sequential review: started new cycle with order {:?} (run counts: {:?})",
            seq_state.current_cycle_order,
            reviewer_ids
                .iter()
                .map(|id| (*id, seq_state.get_run_count(id)))
                .collect::<Vec<_>>()
        );
        if let Some(rejector) = tiebreaker {
            log_msg.push_str(&format!(
                " [tiebreaker: prioritized previous rejector '{}']",
                rejector.as_str()
            ));
        }
        context.log_workflow(&log_msg);
    }

    // Emit round started ONLY for first reviewer in this sequential cycle
    // We handle this here because we pass emit_round_started=false to run_multi_agent_review_with_view
    let is_first_reviewer = seq_state.current_reviewer_index == 0;
    if is_first_reviewer {
        sender.send_review_round_started(ReviewKind::Plan, iteration);
    }

    // Get current reviewer from stored cycle order
    let current_id = seq_state
        .get_current_reviewer()
        .expect("cycle order must be populated after start_new_cycle");
    let reviewer = reviewers
        .iter()
        .find(|r| r.display_id() == current_id.as_str())
        .expect("reviewer must exist in config after validate_reviewer_state");
    let reviewer_id = reviewer.display_id();

    // Increment run count before execution
    seq_state.increment_run_count(reviewer_id);

    sender.send_output(format!(
        "Running reviewer: {} (plan version {}, run #{})",
        reviewer_id,
        seq_state.plan_version,
        seq_state.get_run_count(reviewer_id)
    ));

    // === FAILURE HANDLING LOOP (same pattern as parallel mode) ===
    let mut retry_attempts = 0usize;
    let review = loop {
        // Check for commands before running reviewer
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    context.log_workflow(&format!(
                        "Received interrupt during sequential reviewing: {}",
                        feedback
                    ));
                    sender.send_output("[review] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart {
                        user_feedback: feedback,
                    }));
                }
                WorkflowCommand::Stop => {
                    context.log_workflow("Received stop during sequential reviewing");
                    sender.send_output("[review] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        // Run single reviewer using existing infrastructure (single-element slice)
        // Pass emit_round_started=false to prevent duplicate TUI events
        let batch = run_multi_agent_review_with_context(
            view,
            working_dir,
            config,
            std::slice::from_ref(reviewer),
            sender.clone(),
            iteration,
            context.session_logger.clone(),
            false, // DO NOT emit round_started, we handle it above
            context.actor_ref.clone(),
        )
        .await;

        // Check for cancellation
        let batch = match batch {
            Ok(b) => b,
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                context.log_workflow("Sequential review phase was cancelled");
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        // Handle success case
        if let Some(review) = batch.reviews.into_iter().next() {
            break review;
        }

        // Handle failure case - same logic as parallel mode
        if let Some(failure) = batch.failures.into_iter().next() {
            let max_retries = config.failure_policy.max_retries as usize;
            if retry_attempts < max_retries {
                retry_attempts += 1;
                sender.send_output(format!(
                    "[review:{}] Failed; retrying ({}/{})...",
                    reviewer_id, retry_attempts, max_retries
                ));
                continue;
            }

            // Output bundle path if present
            if let Some(ref path) = failure.bundle_path {
                sender.send_output(format!(
                    "[diagnostics] {}: {}",
                    failure.agent_name,
                    path.display()
                ));
            }

            // Prompt user for recovery decision
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
                    return Ok(Some(WorkflowResult::Stopped));
                }
                AllReviewersFailedDecision::Abort => {
                    context.log_workflow(&format!(
                        "User chose to abort after reviewer {} failed",
                        reviewer_id
                    ));
                    // Dispatch UserAborted command
                    let reason = format!("Reviewer {} failed - user chose to abort", reviewer_id);
                    context
                        .dispatch_command(DomainCommand::UserAborted {
                            reason: reason.clone(),
                        })
                        .await;
                    return Ok(Some(WorkflowResult::Aborted { reason }));
                }
                AllReviewersFailedDecision::Stopped => {
                    context.log_workflow("Workflow stopped during failure decision");
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        // Edge case: both reviews and failures empty (shouldn't happen)
        anyhow::bail!("Review returned no results and no failures");
    };
    // === END FAILURE HANDLING LOOP ===

    // Store current review in last_reviews for potential revision feedback
    // In sequential mode, last_reviews contains only current reviewer's result
    // This is intentional: the revising phase only needs the current rejecting reviewer's feedback
    last_reviews.clear();
    last_reviews.push(review.clone());

    // Write individual feedback file for this reviewer (uses agent-specific filename)
    // This is safe to call repeatedly - each reviewer gets their own file
    let feedback_path = view
        .feedback_path
        .clone()
        .map(|fp| fp.0)
        .unwrap_or_else(|| std::path::PathBuf::from("feedback.md"));
    let _ = write_feedback_files(std::slice::from_ref(&review), &feedback_path);
    // DO NOT call merge_feedback here - we'll do it once at the end when all approve

    let max_iterations = view.max_iterations.map(|m| m.0).unwrap_or(3);

    if review.needs_revision {
        // Record the rejecting reviewer for tiebreaker in next cycle
        seq_state.record_rejection(reviewer_id);
        // Reviewer rejected - transition to revision
        context.log_workflow(&format!(
            "Reviewer {} REJECTED (plan version {}) - transitioning to revision",
            reviewer_id, seq_state.plan_version
        ));
        sender.send_output(format!(
            "[sequential] {} REJECTED - will revise and restart from first reviewer",
            reviewer_id
        ));

        // Dispatch ReviewerRejected command to CQRS actor
        context
            .dispatch_command(DomainCommand::ReviewerRejected {
                reviewer_id: AgentId::from(reviewer_id),
                feedback_path: FeedbackPath::from(feedback_path.clone()),
            })
            .await;

        // Signal round completion (rejected)
        sender.send_review_round_completed(ReviewKind::Plan, iteration, false);

        // Dispatch ReviewCycleCompleted command (rejected)
        context
            .dispatch_command(DomainCommand::ReviewCycleCompleted { approved: false })
            .await;

        // Check iteration limit
        if iteration >= max_iterations {
            // Dispatch PlanningMaxIterationsReached command
            context
                .dispatch_command(DomainCommand::PlanningMaxIterationsReached)
                .await;

            return handle_max_iterations_with_view(
                view,
                &context.session_logger,
                sender,
                approval_rx,
                control_rx,
                last_reviews,
                context,
            )
            .await;
        }

        // Transition is handled by ReviewCycleCompleted event
    } else {
        // Reviewer approved - record approval and accumulate review for summary
        let serializable_review = SerializableReviewResult {
            agent_name: review.agent_name.clone(),
            needs_revision: review.needs_revision,
            feedback: review.feedback.clone(),
            summary: review.summary.clone(),
        };
        seq_state.record_approval(reviewer_id, &serializable_review);

        context.log_workflow(&format!(
            "Reviewer {} APPROVED (plan version {})",
            reviewer_id, seq_state.plan_version
        ));
        sender.send_output(format!(
            "[sequential] {} APPROVED (version {})",
            reviewer_id, seq_state.plan_version
        ));

        // Dispatch ReviewerApproved command to CQRS actor
        context
            .dispatch_command(DomainCommand::ReviewerApproved {
                reviewer_id: AgentId::from(reviewer_id),
            })
            .await;

        // Extract reviewer IDs for all_approved check (avoids circular dependency)
        let reviewer_ids: Vec<&str> = reviewers.iter().map(|r| r.display_id()).collect();

        // Check if all reviewers have approved current version
        if seq_state.all_approved(&reviewer_ids) {
            // All approved the same version - complete!
            context.log_workflow("All reviewers approved - plan complete!");
            sender.send_output("[sequential] All reviewers approved - plan complete!".to_string());

            // Signal round completion (approved)
            sender.send_review_round_completed(ReviewKind::Plan, iteration, true);

            // Dispatch ReviewCycleCompleted command (approved)
            context
                .dispatch_command(DomainCommand::ReviewCycleCompleted { approved: true })
                .await;

            // NOW write merged feedback with ALL accumulated reviews
            let all_reviews = seq_state.get_accumulated_reviews_for_summary();
            let _ = merge_feedback(&all_reviews, &feedback_path);

            // Pass ALL accumulated reviews to summary generation
            let review_phase_name = format!("Reviewing #{}", iteration);
            phases::spawn_summary_generation(
                review_phase_name,
                view,
                working_dir,
                config,
                sender.clone(),
                Some(&all_reviews),
                context.session_logger.clone(),
            );

            // Transition is handled by ReviewCycleCompleted event
        } else {
            // More reviewers to go - advance to next in stored cycle order
            seq_state.advance_to_next_reviewer();
            let next_id = seq_state
                .get_current_reviewer()
                .map(|id| id.as_str())
                .unwrap_or("(end of cycle)");
            context.log_workflow(&format!(
                "Advancing to reviewer {}/{}: {}",
                seq_state.current_reviewer_index + 1,
                reviewers.len(),
                next_id
            ));
            // Stay in Reviewing phase - next workflow loop iteration will process next reviewer
        }
    }

    Ok(None)
}
