//! Workflow completion handling.

use super::WorkflowResult;
use crate::app::util::{build_approval_summary, log_workflow};
use crate::git_worktree;
use crate::state::State;
use crate::tui::{SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

pub async fn handle_completion(
    state: &State,
    working_dir: &Path,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
) -> Result<WorkflowResult> {
    log_workflow(working_dir, ">>> Plan complete - requesting user approval");

    sender.send_output("".to_string());

    // Output merge instructions if using a worktree
    if let Some(ref wt_state) = state.worktree_info {
        let info = git_worktree::WorktreeInfo {
            worktree_path: wt_state.worktree_path.clone(),
            branch_name: wt_state.branch_name.clone(),
            source_branch: wt_state.source_branch.clone(),
            original_dir: wt_state.original_dir.clone(),
            has_submodules: false, // Don't re-check at completion
        };
        let instructions = git_worktree::generate_merge_instructions(&info);
        for line in instructions.lines() {
            sender.send_output(line.to_string());
        }
    }

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
        tokio::select! {
            Some(cmd) = control_rx.recv() => {
                match cmd {
                    WorkflowCommand::Stop => {
                        log_workflow(working_dir, "Stop command received during approval wait");
                        sender.send_output("[workflow] Stopping during approval...".to_string());
                        return Ok(WorkflowResult::Stopped);
                    }
                    WorkflowCommand::Interrupt { feedback } => {
                        log_workflow(working_dir, &format!("Interrupt received during approval: {}", feedback));
                        sender.send_output("[workflow] Interrupted during approval".to_string());
                        return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                    }
                }
            }
            response = approval_rx.recv() => {
                match response {
                    Some(UserApprovalResponse::Accept) => {
                        log_workflow(working_dir, "User ACCEPTED the plan");
                        sender.send_output("[planning] User accepted the plan!".to_string());
                        return Ok(WorkflowResult::Accepted);
                    }
                    Some(UserApprovalResponse::Implement) => {
                        log_workflow(working_dir, "User requested IMPLEMENTATION");
                        sender.send_output("[planning] Starting implementation workflow...".to_string());
                        return Ok(WorkflowResult::ImplementationRequested);
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
                    Some(UserApprovalResponse::WorkflowFailureRetry)
                    | Some(UserApprovalResponse::WorkflowFailureStop)
                    | Some(UserApprovalResponse::WorkflowFailureAbort) => {
                        log_workflow(
                            working_dir,
                            "Received workflow failure response while awaiting plan approval, ignoring",
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
    }
}
