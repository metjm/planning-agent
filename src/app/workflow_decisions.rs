use crate::app::workflow::WorkflowResult;
use crate::phases::ReviewResult;
use crate::session_logger::{LogCategory, LogLevel, SessionLogger};
use crate::state::{Phase, State};
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReviewDecision {
    Retry,
    Continue,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanFailureDecision {
    Retry,
    Continue,
    Abort,
    Stopped,
}

/// Decision for handling when all reviewers fail (no partial reviews available).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AllReviewersFailedDecision {
    /// User chose to retry all reviewers.
    Retry,
    /// User chose to stop and save state for later resume.
    Stop,
    /// User chose to abort the workflow.
    Abort,
    /// Workflow was stopped via control channel.
    Stopped,
}

/// Decision for handling generic workflow failures (agent crashes, timeouts, etc.).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorkflowFailureDecision {
    /// User chose to retry the failed operation.
    Retry,
    /// User chose to stop and save state for later resume.
    Stop,
    /// User chose to abort the workflow.
    Abort,
    /// Workflow was stopped via control channel.
    Stopped,
}

/// Helper to log workflow decision messages.
fn log_decision(logger: &SessionLogger, message: &str) {
    logger.log(LogLevel::Info, LogCategory::Workflow, message);
}

pub async fn wait_for_review_decision(
    session_logger: &Arc<SessionLogger>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> ReviewDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(session_logger, "Stop command received during review decision wait");
                    return ReviewDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                return match response {
                    Some(UserApprovalResponse::ReviewRetry) => ReviewDecision::Retry,
                    Some(UserApprovalResponse::ReviewContinue) => ReviewDecision::Continue,
                    Some(UserApprovalResponse::Accept) | Some(UserApprovalResponse::Implement) => {
                        log_decision(session_logger, "Received plan approval while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::Decline(_)) => {
                        log_decision(session_logger, "Received plan decline while awaiting review decision, treating as retry");
                        ReviewDecision::Retry
                    }
                    Some(UserApprovalResponse::PlanGenerationRetry) => {
                        log_decision(session_logger, "Received PlanGenerationRetry while awaiting review decision, treating as retry");
                        ReviewDecision::Retry
                    }
                    Some(UserApprovalResponse::PlanGenerationContinue) => {
                        log_decision(session_logger, "Received PlanGenerationContinue while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_decision(session_logger, "Received AbortWorkflow while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_decision(session_logger, "Received ProceedWithoutApproval while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::ContinueReviewing) => {
                        log_decision(session_logger, "Received ContinueReviewing while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::WorkflowFailureRetry)
                    | Some(UserApprovalResponse::WorkflowFailureStop)
                    | Some(UserApprovalResponse::WorkflowFailureAbort) => {
                        log_decision(session_logger, "Received workflow failure response while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    None => {
                        log_decision(session_logger, "Review decision channel closed, treating as continue");
                        ReviewDecision::Continue
                    }
                };
            }
        }
    }
}

pub async fn wait_for_plan_failure_decision(
    session_logger: &Arc<SessionLogger>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    plan_exists: bool,
) -> PlanFailureDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(session_logger, "Stop command received during plan failure decision wait");
                    return PlanFailureDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::PlanGenerationRetry) => {
                        log_decision(session_logger, "User chose to retry plan generation");
                        return PlanFailureDecision::Retry;
                    }
                    Some(UserApprovalResponse::PlanGenerationContinue) => {
                        if plan_exists {
                            log_decision(session_logger, "User chose to continue with existing plan");
                            return PlanFailureDecision::Continue;
                        } else {
                            log_decision(session_logger, "User chose continue but no plan exists, treating as retry");
                            return PlanFailureDecision::Retry;
                        }
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_decision(session_logger, "User chose to abort workflow");
                        return PlanFailureDecision::Abort;
                    }
                    Some(other) => {
                        log_decision(session_logger, &format!("Ignoring unexpected response {:?} during plan failure prompt", other));
                        continue;
                    }
                    None => {
                        log_decision(session_logger, "Approval channel closed during plan failure prompt - aborting");
                        return PlanFailureDecision::Abort;
                    }
                }
            }
        }
    }
}

/// Waits for user decision when all reviewers have failed.
/// This is different from `wait_for_review_decision` because there are no partial reviews
/// to continue with - the only options are retry, stop, or abort.
pub async fn wait_for_all_reviewers_failed_decision(
    session_logger: &Arc<SessionLogger>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> AllReviewersFailedDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(session_logger, "Stop command received during all reviewers failed decision wait");
                    return AllReviewersFailedDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::ReviewRetry) => {
                        log_decision(session_logger, "User chose to retry all reviewers");
                        return AllReviewersFailedDecision::Retry;
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_decision(session_logger, "User chose to abort workflow after all reviewers failed");
                        return AllReviewersFailedDecision::Abort;
                    }
                    // Stop and save for later resume
                    Some(UserApprovalResponse::Accept) => {
                        log_decision(session_logger, "User chose to stop and save state");
                        return AllReviewersFailedDecision::Stop;
                    }
                    Some(other) => {
                        log_decision(session_logger, &format!("Ignoring unexpected response {:?} during all reviewers failed prompt", other));
                        continue;
                    }
                    None => {
                        log_decision(session_logger, "Approval channel closed during all reviewers failed prompt - aborting");
                        return AllReviewersFailedDecision::Abort;
                    }
                }
            }
        }
    }
}

/// Waits for user decision when a generic workflow failure occurs.
/// Used for agent crashes, timeouts, and other non-reviewer failures.
pub async fn wait_for_workflow_failure_decision(
    session_logger: &Arc<SessionLogger>,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> WorkflowFailureDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(session_logger, "Stop command received during workflow failure decision wait");
                    return WorkflowFailureDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::WorkflowFailureRetry) => {
                        log_decision(session_logger, "User chose to retry after workflow failure");
                        return WorkflowFailureDecision::Retry;
                    }
                    Some(UserApprovalResponse::WorkflowFailureStop) => {
                        log_decision(session_logger, "User chose to stop and save after workflow failure");
                        return WorkflowFailureDecision::Stop;
                    }
                    Some(UserApprovalResponse::WorkflowFailureAbort) => {
                        log_decision(session_logger, "User chose to abort after workflow failure");
                        return WorkflowFailureDecision::Abort;
                    }
                    Some(other) => {
                        log_decision(session_logger, &format!("Ignoring unexpected response {:?} during workflow failure prompt", other));
                        continue;
                    }
                    None => {
                        log_decision(session_logger, "Approval channel closed during workflow failure prompt - aborting");
                        return WorkflowFailureDecision::Abort;
                    }
                }
            }
        }
    }
}

pub async fn handle_max_iterations(
    state: &mut State,
    session_logger: &Arc<SessionLogger>,
    state_path: &Path,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &[ReviewResult],
) -> Result<Option<WorkflowResult>> {
    log_decision(session_logger, "Max iterations reached - prompting user");
    sender.send_output("[planning] Max iterations reached".to_string());
    sender.send_output("[planning] Awaiting your decision...".to_string());

    // Note: build_max_iterations_summary still needs working_dir for plan_path construction
    // We pass state.plan_file directly since it's already an absolute path
    let summary = build_max_iterations_summary_from_state(state, last_reviews);
    sender.send_max_iterations_reached(summary);

    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(session_logger, "Stop command received during max iterations wait");
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_decision(session_logger, "User chose to proceed without AI approval");
                        sender.send_output("[planning] Proceeding without AI approval...".to_string());
                        state.approval_overridden = true;
                        state.transition(Phase::Complete)?;
                        state.set_updated_at();
                        state.save(state_path)?;
                        return Ok(None);
                    }
                    Some(UserApprovalResponse::ContinueReviewing) => {
                        log_decision(session_logger, "User chose to continue reviewing");
                        sender.send_output("[planning] Continuing with another review cycle...".to_string());
                        state.max_iterations += 1;
                        state.transition(Phase::Revising)?;
                        state.set_updated_at();
                        state.save(state_path)?;
                        return Ok(None);
                    }
                    Some(UserApprovalResponse::Decline(feedback)) => {
                        log_decision(session_logger, &format!("User declined with feedback: {}", feedback));
                        sender.send_output(format!("[planning] Restarting with feedback: {}", feedback));
                        return Ok(Some(WorkflowResult::NeedsRestart { user_feedback: feedback }));
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_decision(session_logger, "User chose to abort workflow");
                        sender.send_output("[planning] Workflow aborted by user".to_string());
                        return Ok(Some(WorkflowResult::Aborted { reason: "User aborted workflow at max iterations".to_string() }));
                    }
                    Some(other) => {
                        log_decision(session_logger, &format!("Ignoring unexpected response {:?} during max iterations prompt", other));
                        continue;
                    }
                    None => {
                        log_decision(session_logger, "Approval channel closed - aborting");
                        return Ok(Some(WorkflowResult::Aborted { reason: "Approval channel closed".to_string() }));
                    }
                }
            }
        }
    }
}

/// Build max iterations summary using state's plan_file (already an absolute path).
fn build_max_iterations_summary_from_state(state: &State, last_reviews: &[ReviewResult]) -> String {
    use crate::app::util::truncate_for_summary;

    let plan_path = &state.plan_file;

    let mut summary = format!(
        "The plan has been reviewed {} times but has not been approved by AI.\n\nPlan file: {}\n\n",
        state.iteration,
        plan_path.display()
    );

    if let Some(ref status) = state.last_feedback_status {
        summary.push_str(&format!("Last review verdict: {:?}\n\n", status));
    }

    if !last_reviews.is_empty() {
        // New top section: Review Summary with verdict grouping
        summary.push_str("---\n\n## Review Summary\n\n");

        // Count verdicts
        let needs_revision_count = last_reviews.iter().filter(|r| r.needs_revision).count();
        let approved_count = last_reviews.len() - needs_revision_count;

        summary.push_str(&format!(
            "**{} reviewer(s):** {} needs revision, {} approved\n\n",
            last_reviews.len(),
            needs_revision_count,
            approved_count
        ));

        // Group reviewers by verdict
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

        // Per-agent summary bullets
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

        // Preview section: concise cut-off view
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

        // Full feedback section: complete review content
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
