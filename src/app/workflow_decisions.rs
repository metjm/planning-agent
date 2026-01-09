use crate::app::util::{build_max_iterations_summary, log_workflow};
use crate::app::workflow::WorkflowResult;
use crate::phases::ReviewResult;
use crate::state::{Phase, State};
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
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

pub async fn wait_for_review_decision(
    working_dir: &Path,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> ReviewDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_workflow(working_dir, "Stop command received during review decision wait");
                    return ReviewDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                return match response {
                    Some(UserApprovalResponse::ReviewRetry) => ReviewDecision::Retry,
                    Some(UserApprovalResponse::ReviewContinue) => ReviewDecision::Continue,
                    Some(UserApprovalResponse::Accept) => {
                        log_workflow(working_dir, "Received plan approval while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::Decline(_)) => {
                        log_workflow(working_dir, "Received plan decline while awaiting review decision, treating as retry");
                        ReviewDecision::Retry
                    }
                    Some(UserApprovalResponse::PlanGenerationRetry) => {
                        log_workflow(working_dir, "Received PlanGenerationRetry while awaiting review decision, treating as retry");
                        ReviewDecision::Retry
                    }
                    Some(UserApprovalResponse::PlanGenerationContinue) => {
                        log_workflow(working_dir, "Received PlanGenerationContinue while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_workflow(working_dir, "Received AbortWorkflow while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_workflow(working_dir, "Received ProceedWithoutApproval while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    Some(UserApprovalResponse::ContinueReviewing) => {
                        log_workflow(working_dir, "Received ContinueReviewing while awaiting review decision, treating as continue");
                        ReviewDecision::Continue
                    }
                    None => {
                        log_workflow(working_dir, "Review decision channel closed, treating as continue");
                        ReviewDecision::Continue
                    }
                };
            }
        }
    }
}

pub async fn wait_for_plan_failure_decision(
    working_dir: &Path,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    plan_exists: bool,
) -> PlanFailureDecision {
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_workflow(working_dir, "Stop command received during plan failure decision wait");
                    return PlanFailureDecision::Stopped;
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::PlanGenerationRetry) => {
                        log_workflow(working_dir, "User chose to retry plan generation");
                        return PlanFailureDecision::Retry;
                    }
                    Some(UserApprovalResponse::PlanGenerationContinue) => {
                        if plan_exists {
                            log_workflow(working_dir, "User chose to continue with existing plan");
                            return PlanFailureDecision::Continue;
                        } else {
                            log_workflow(working_dir, "User chose continue but no plan exists, treating as retry");
                            return PlanFailureDecision::Retry;
                        }
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_workflow(working_dir, "User chose to abort workflow");
                        return PlanFailureDecision::Abort;
                    }
                    Some(other) => {
                        log_workflow(working_dir, &format!("Ignoring unexpected response {:?} during plan failure prompt", other));
                        continue;
                    }
                    None => {
                        log_workflow(working_dir, "Approval channel closed during plan failure prompt - aborting");
                        return PlanFailureDecision::Abort;
                    }
                }
            }
        }
    }
}

pub async fn handle_max_iterations(
    state: &mut State,
    working_dir: &Path,
    state_path: &Path,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &[ReviewResult],
) -> Result<Option<WorkflowResult>> {
    log_workflow(working_dir, "Max iterations reached - prompting user");
    sender.send_output("[planning] Max iterations reached".to_string());
    sender.send_output("[planning] Awaiting your decision...".to_string());

    let summary = build_max_iterations_summary(state, working_dir, last_reviews);
    sender.send_max_iterations_reached(summary);

    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_workflow(working_dir, "Stop command received during max iterations wait");
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_workflow(working_dir, "User chose to proceed without AI approval");
                        sender.send_output("[planning] Proceeding without AI approval...".to_string());
                        state.approval_overridden = true;
                        state.transition(Phase::Complete)?;
                        state.set_updated_at();
                        state.save(state_path)?;
                        return Ok(None);
                    }
                    Some(UserApprovalResponse::ContinueReviewing) => {
                        log_workflow(working_dir, "User chose to continue reviewing");
                        sender.send_output("[planning] Continuing with another review cycle...".to_string());
                        state.max_iterations += 1;
                        state.transition(Phase::Revising)?;
                        state.set_updated_at();
                        state.save(state_path)?;
                        return Ok(None);
                    }
                    Some(UserApprovalResponse::Decline(feedback)) => {
                        log_workflow(working_dir, &format!("User declined with feedback: {}", feedback));
                        sender.send_output(format!("[planning] Restarting with feedback: {}", feedback));
                        return Ok(Some(WorkflowResult::NeedsRestart { user_feedback: feedback }));
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_workflow(working_dir, "User chose to abort workflow");
                        sender.send_output("[planning] Workflow aborted by user".to_string());
                        return Ok(Some(WorkflowResult::Aborted { reason: "User aborted workflow at max iterations".to_string() }));
                    }
                    Some(other) => {
                        log_workflow(working_dir, &format!("Ignoring unexpected response {:?} during max iterations prompt", other));
                        continue;
                    }
                    None => {
                        log_workflow(working_dir, "Approval channel closed - aborting");
                        return Ok(Some(WorkflowResult::Aborted { reason: "Approval channel closed".to_string() }));
                    }
                }
            }
        }
    }
}
