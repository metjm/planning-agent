//! Planning phase execution.

use super::{dispatch_domain_command, WorkflowResult};
use crate::app::util::build_plan_failure_summary;
use crate::app::workflow_common::plan_file_has_content;
use crate::app::workflow_decisions::{wait_for_plan_failure_decision, PlanFailureDecision};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::types::PlanPath;
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::{self, run_planning_phase_with_context};
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{CancellationError, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use ractor::ActorRef;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub async fn run_planning_phase(
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    session_logger: Arc<SessionLogger>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
) -> Result<Option<WorkflowResult>> {
    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        ">>> ENTERING Planning phase",
    );
    sender.send_phase_started("Planning".to_string());
    sender.send_output("".to_string());
    sender.send_output("=== PLANNING PHASE ===".to_string());
    let feature_name = view.feature_name().map(|f| f.0.as_str()).unwrap_or("");
    sender.send_output(format!("Feature: {}", feature_name));
    sender.send_output(format!("Agent: {}", config.workflow.planning.agent));
    let plan_path = view.plan_path().map(|p| p.0.clone()).unwrap_or_default();
    sender.send_output(format!("Plan file: {}", plan_path.display()));

    loop {
        // Check for commands before starting planning
        if let Ok(cmd) = control_rx.try_recv() {
            match cmd {
                WorkflowCommand::Interrupt { feedback } => {
                    session_logger.log(
                        LogLevel::Info,
                        LogCategory::Workflow,
                        &format!("Received interrupt during planning: {}", feedback),
                    );
                    sender.send_output("[planning] Interrupted by user".to_string());
                    return Ok(Some(WorkflowResult::NeedsRestart {
                        user_feedback: feedback,
                    }));
                }
                WorkflowCommand::Stop => {
                    session_logger.log(
                        LogLevel::Info,
                        LogCategory::Workflow,
                        "Received stop during planning",
                    );
                    sender.send_output("[planning] Stopping...".to_string());
                    return Ok(Some(WorkflowResult::Stopped));
                }
            }
        }

        session_logger.log(
            LogLevel::Info,
            LogCategory::Workflow,
            "Calling run_planning_phase_with_context...",
        );
        let planning_result = run_planning_phase_with_context(
            view,
            working_dir,
            config,
            sender.clone(),
            session_logger.clone(),
            actor_ref.clone(),
        )
        .await;

        match planning_result {
            Ok(()) => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    "run_planning_phase_with_context completed",
                );

                // Use content-based check instead of exists() for pre-created files
                if !plan_file_has_content(&plan_path) {
                    session_logger.log(
                        LogLevel::Info,
                        LogCategory::Workflow,
                        "ERROR: Plan file has no content!",
                    );
                    sender.send_output(
                        "[error] Plan file has no content - planning agent may have failed"
                            .to_string(),
                    );

                    // Prompt user for decision
                    let summary = build_plan_failure_summary(
                        "Plan file has no content - planning agent may have failed",
                        &plan_path,
                        false,
                    );
                    sender.send_plan_generation_failed(summary);

                    match wait_for_plan_failure_decision(
                        &session_logger,
                        approval_rx,
                        control_rx,
                        false,
                    )
                    .await
                    {
                        PlanFailureDecision::Retry => {
                            sender
                                .send_output("[planning] Retrying plan generation...".to_string());
                            continue;
                        }
                        PlanFailureDecision::Continue => {
                            // This shouldn't happen since plan has no content, but handle it
                            sender.send_output(
                                "[planning] Plan file has no content to continue with. Retrying..."
                                    .to_string(),
                            );
                            continue;
                        }
                        PlanFailureDecision::Abort => {
                            session_logger.log(
                                LogLevel::Info,
                                LogCategory::Workflow,
                                "User aborted after plan file empty",
                            );
                            // Dispatch UserAborted command
                            let reason = "User aborted: plan file has no content".to_string();
                            dispatch_domain_command(
                                &actor_ref,
                                DomainCommand::UserAborted {
                                    reason: reason.clone(),
                                },
                                &session_logger,
                            )
                            .await;
                            return Ok(Some(WorkflowResult::Aborted { reason }));
                        }
                        PlanFailureDecision::Stopped => {
                            session_logger.log(
                                LogLevel::Info,
                                LogCategory::Workflow,
                                "Workflow stopped during plan failure decision",
                            );
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
                    session_logger.log(
                        LogLevel::Info,
                        LogCategory::Workflow,
                        "Planning phase was cancelled",
                    );
                    // Re-throw to be handled by caller
                    return Err(e);
                }

                let error_msg = format!("{}", e);
                session_logger.log(
                    LogLevel::Error,
                    LogCategory::Workflow,
                    &format!("Planning phase error: {}", error_msg),
                );
                sender.send_output(format!("[error] Planning failed: {}", error_msg));

                // Use content-based check instead of exists() for pre-created files
                let plan_has_content = plan_file_has_content(&plan_path);
                let summary = build_plan_failure_summary(&error_msg, &plan_path, plan_has_content);
                sender.send_plan_generation_failed(summary);

                match wait_for_plan_failure_decision(
                    &session_logger,
                    approval_rx,
                    control_rx,
                    plan_has_content,
                )
                .await
                {
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
                            sender.send_output(
                                "[planning] Plan file has no content to continue with. Retrying..."
                                    .to_string(),
                            );
                            continue;
                        }
                    }
                    PlanFailureDecision::Abort => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "User aborted after planning error",
                        );
                        // Dispatch UserAborted command
                        let reason = format!("User aborted: {}", error_msg);
                        dispatch_domain_command(
                            &actor_ref,
                            DomainCommand::UserAborted {
                                reason: reason.clone(),
                            },
                            &session_logger,
                        )
                        .await;
                        return Ok(Some(WorkflowResult::Aborted { reason }));
                    }
                    PlanFailureDecision::Stopped => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "Workflow stopped during plan failure decision",
                        );
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                }
            }
        }
    }

    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        "Transitioning: Planning -> Reviewing",
    );
    dispatch_domain_command(
        &actor_ref,
        DomainCommand::PlanningCompleted {
            plan_path: PlanPath(plan_path.clone()),
        },
        &session_logger,
    )
    .await;
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    phases::spawn_summary_generation(
        "Planning".to_string(),
        view,
        working_dir,
        config,
        sender.clone(),
        None,
        session_logger.clone(),
    );

    Ok(None)
}
