//! Workflow completion handling.

use super::WorkflowResult;
use crate::app::util::build_approval_summary;
use crate::domain::view::WorkflowView;
use crate::git_worktree;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Helper to log completion messages.
fn log_completion(logger: &SessionLogger, message: &str) {
    logger.log(LogLevel::Info, LogCategory::Workflow, message);
}

pub async fn handle_completion(
    view: &WorkflowView,
    session_logger: &Arc<SessionLogger>,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> Result<WorkflowResult> {
    log_completion(
        session_logger,
        ">>> Plan complete - requesting user approval",
    );

    sender.send_output("".to_string());

    // Output merge instructions if using a worktree
    if let Some(wt_state) = view.worktree_info() {
        let info = git_worktree::WorktreeInfo {
            worktree_path: wt_state.worktree_path().to_path_buf(),
            branch_name: wt_state.branch_name().to_string(),
            source_branch: wt_state.source_branch().map(|s| s.to_string()),
            original_dir: wt_state.original_dir().to_path_buf(),
            has_submodules: false, // Don't re-check at completion
        };
        let instructions = git_worktree::generate_merge_instructions(&info);
        for line in instructions.lines() {
            sender.send_output(line.to_string());
        }
    }

    // view.plan_path() is now an absolute path (in ~/.planning-agent/plans/)
    let plan_path = view
        .plan_path()
        .expect("plan_path must be set before completion")
        .0
        .clone();
    let iteration = view
        .iteration()
        .expect("iteration must be set before completion")
        .0;

    if view.approval_overridden() {
        sender.send_output("=== PROCEEDING WITHOUT AI APPROVAL ===".to_string());
        sender.send_output("User chose to proceed after max iterations".to_string());
        sender.send_output("Waiting for your final decision...".to_string());

        let summary = build_approval_summary(&plan_path, true, iteration);
        sender.send_user_override_approval(summary);
    } else {
        sender.send_output("=== PLAN APPROVED BY AI ===".to_string());
        sender.send_output(format!("Completed after {} iteration(s)", iteration));
        sender.send_output("Waiting for your approval...".to_string());

        let summary = build_approval_summary(&plan_path, false, iteration);
        sender.send_approval_request(summary);
    };

    log_completion(session_logger, "Waiting for user approval response...");
    loop {
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                match cmd {
                    WorkflowCommand::Stop => {
                        log_completion(session_logger, "Stop command received during approval wait");
                        sender.send_output("[workflow] Stopping during approval...".to_string());
                        return Ok(WorkflowResult::Stopped);
                    }
                    WorkflowCommand::Interrupt { feedback } => {
                        log_completion(session_logger, &format!("Interrupt received during approval: {}", feedback));
                        sender.send_output("[workflow] Interrupted during approval".to_string());
                        return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                    }
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::Accept) => {
                        log_completion(session_logger, "User ACCEPTED the plan");
                        sender.send_output("[planning] User accepted the plan!".to_string());
                        return Ok(WorkflowResult::Accepted);
                    }
                    Some(UserApprovalResponse::Implement) => {
                        log_completion(session_logger, "User requested IMPLEMENTATION");
                        sender.send_output("[planning] Starting implementation workflow...".to_string());
                        return Ok(WorkflowResult::ImplementationRequested);
                    }
                    Some(UserApprovalResponse::Decline(feedback)) => {
                        log_completion(
                            session_logger,
                            &format!("User DECLINED with feedback: {}", feedback),
                        );
                        sender.send_output(format!("[planning] User requested changes: {}", feedback));
                        return Ok(WorkflowResult::NeedsRestart {
                            user_feedback: feedback,
                        });
                    }
                    Some(UserApprovalResponse::ReviewRetry)
                    | Some(UserApprovalResponse::ReviewContinue) => {
                        log_completion(
                            session_logger,
                            "Received review decision while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::PlanGenerationRetry) => {
                        log_completion(
                            session_logger,
                            "Received PlanGenerationRetry while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::PlanGenerationContinue) => {
                        log_completion(
                            session_logger,
                            "Received PlanGenerationContinue while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::AbortWorkflow) => {
                        log_completion(
                            session_logger,
                            "Received AbortWorkflow while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::ProceedWithoutApproval) => {
                        log_completion(
                            session_logger,
                            "Received ProceedWithoutApproval while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::ContinueReviewing(_)) => {
                        log_completion(
                            session_logger,
                            "Received ContinueReviewing while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    Some(UserApprovalResponse::WorkflowFailureRetry)
                    | Some(UserApprovalResponse::WorkflowFailureStop)
                    | Some(UserApprovalResponse::WorkflowFailureAbort) => {
                        log_completion(
                            session_logger,
                            "Received workflow failure response while awaiting plan approval, ignoring",
                        );
                        continue;
                    }
                    None => {
                        log_completion(session_logger, "Approval channel closed - treating as accept");
                        return Ok(WorkflowResult::Accepted);
                    }
                }
            }
        }
    }
}
