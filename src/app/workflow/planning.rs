//! Planning phase execution.

use super::WorkflowResult;
use crate::app::util::{build_plan_failure_summary, log_workflow};
use crate::app::workflow_common::plan_file_has_content;
use crate::app::workflow_decisions::{wait_for_plan_failure_decision, PlanFailureDecision};
use crate::config::WorkflowConfig;
use crate::phases::{self, run_planning_phase_with_context};
use crate::state::{Phase, State};
use crate::tui::{CancellationError, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

pub async fn run_planning_phase(
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
