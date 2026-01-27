use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
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

/// Identifies which iterative phase reached max iterations.
/// Used for logging, summary generation, and future extensibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterativePhase {
    /// Planning/review phase
    Planning,
    /// Implementation/implementation-review phase
    Implementation,
    // Future: Verification, Testing, etc.
}

impl IterativePhase {
    /// Returns the display name for logging and summaries
    pub fn display_name(&self) -> &'static str {
        match self {
            IterativePhase::Planning => "planning",
            IterativePhase::Implementation => "implementation",
        }
    }
}

/// Decision made by user when max iterations are reached.
/// This enum is phase-agnostic - each phase interprets it in its own context.
#[derive(Debug, Clone, PartialEq)]
pub enum MaxIterationsDecision {
    /// Accept current state without further review/cycles
    ProceedWithoutApproval,
    /// Run another iteration cycle with the specified number of additional iterations
    Continue(u32),
    /// Restart with user-provided feedback
    RestartWithFeedback(String),
    /// Abort the workflow
    Abort,
    /// Workflow was stopped via control channel
    Stopped,
}

/// Helper to log workflow decision messages.
fn log_decision(logger: &SessionLogger, message: &str) {
    logger.log(LogLevel::Info, LogCategory::Workflow, message);
}

/// Awaits user decision when max iterations are reached.
/// This is a low-level function that handles TUI interaction only.
/// The caller is responsible for interpreting the decision and applying state changes.
///
/// # Arguments
/// * `phase` - Which iterative phase reached max iterations
/// * `session_logger` - For logging
/// * `sender` - For sending TUI events
/// * `approval_rx` - Channel to receive user responses
/// * `control_rx` - Channel to receive control commands (stop)
/// * `summary` - Pre-built summary text to display in the modal
pub async fn await_max_iterations_decision(
    phase: IterativePhase,
    session_logger: &Arc<SessionLogger>,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    summary: String,
) -> Result<MaxIterationsDecision> {
    let phase_name = phase.display_name();
    log_decision(
        session_logger,
        &format!("[{}] Max iterations reached - prompting user", phase_name),
    );
    sender.send_output(format!("[{}] Max iterations reached", phase_name));
    sender.send_output(format!("[{}] Awaiting your decision...", phase_name));

    sender.send_max_iterations_reached(phase, summary);

    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                if matches!(cmd, WorkflowCommand::Stop) {
                    log_decision(
                        session_logger,
                        &format!("[{}] Stop command received during max iterations wait", phase_name),
                    );
                    return Ok(MaxIterationsDecision::Stopped);
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_decision(
                            session_logger,
                            &format!("[{}] User chose to proceed without approval", phase_name),
                        );
                        return Ok(MaxIterationsDecision::ProceedWithoutApproval);
                    }
                    Some(UserApprovalResponse::ContinueReviewing(additional)) => {
                        let count = if additional == 0 { 1 } else { additional };
                        log_decision(
                            session_logger,
                            &format!("[{}] User chose to continue with {} additional iteration(s)", phase_name, count),
                        );
                        return Ok(MaxIterationsDecision::Continue(count));
                    }
                    Some(UserApprovalResponse::Decline(feedback)) => {
                        log_decision(
                            session_logger,
                            &format!("[{}] User declined with feedback: {}", phase_name, feedback),
                        );
                        return Ok(MaxIterationsDecision::RestartWithFeedback(feedback));
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_decision(
                            session_logger,
                            &format!("[{}] User chose to abort workflow", phase_name),
                        );
                        return Ok(MaxIterationsDecision::Abort);
                    }
                    Some(other) => {
                        log_decision(
                            session_logger,
                            &format!("[{}] Ignoring unexpected response {:?}", phase_name, other),
                        );
                        continue;
                    }
                    None => {
                        log_decision(
                            session_logger,
                            &format!("[{}] Approval channel closed - aborting", phase_name),
                        );
                        return Ok(MaxIterationsDecision::Abort);
                    }
                }
            }
        }
    }
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
                    Some(UserApprovalResponse::ContinueReviewing(_)) => {
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
