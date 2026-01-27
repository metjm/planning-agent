//! Revising phase execution.

use super::{dispatch_domain_command, WorkflowResult};
use crate::app::util::build_workflow_failure_summary;
use crate::app::workflow_decisions::{wait_for_workflow_failure_decision, WorkflowFailureDecision};
use crate::config::WorkflowConfig;
use crate::domain::actor::WorkflowMessage;
use crate::domain::failure::{FailureContext, FailureKind};
use crate::domain::review::ReviewMode;
use crate::domain::types::PhaseLabel;
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::{self, run_revision_phase_with_context};
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{CancellationError, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use ractor::ActorRef;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub async fn run_revising_phase(
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
    session_logger: Arc<SessionLogger>,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
) -> Result<Option<WorkflowResult>> {
    // Check for commands before starting revision
    if let Ok(cmd) = control_rx.try_recv() {
        match cmd {
            WorkflowCommand::Interrupt { feedback } => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    &format!("Received interrupt during revising: {}", feedback),
                );
                sender.send_output("[revision] Interrupted by user".to_string());
                return Err(CancellationError { feedback }.into());
            }
            WorkflowCommand::Stop => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    "Received stop during revising",
                );
                sender.send_output("[revision] Stopping...".to_string());
                return Ok(Some(WorkflowResult::Stopped));
            }
        }
    }

    // Extract iteration from view (Iteration is a newtype wrapping u32)
    let iteration = view.iteration().map(|i| i.0).unwrap_or(1);

    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        &format!(">>> ENTERING Revising phase (iteration {})", iteration),
    );
    sender.send_phase_started("Revising".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!("=== REVISION PHASE (Iteration {}) ===", iteration));
    // Revision uses the planning agent for session continuity
    sender.send_output(format!(
        "Agent: {} (planning agent)",
        config.workflow.planning.agent
    ));

    let max_retries = config.failure_policy.max_retries() as usize;
    let mut retry_attempts = 0usize;

    loop {
        session_logger.log(
            LogLevel::Info,
            LogCategory::Workflow,
            "Calling run_revision_phase_with_context...",
        );
        let revision_result = run_revision_phase_with_context(
            view,
            working_dir,
            config,
            last_reviews,
            sender.clone(),
            iteration,
            session_logger.clone(),
            actor_ref.clone(),
        )
        .await;

        match revision_result {
            Ok(()) => {
                // Success - continue with the rest of the phase
                break;
            }
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    "Revising phase was cancelled",
                );
                return Err(e);
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    &format!("Revising phase failed: {}", error_msg),
                );
                sender.send_output(format!("[revision] Failed: {}", error_msg));

                // Check if we can auto-retry
                if retry_attempts < max_retries {
                    retry_attempts += 1;
                    sender.send_output(format!(
                        "[revision] Retrying ({}/{})...",
                        retry_attempts, max_retries
                    ));
                    continue;
                }

                // Max retries reached - prompt user for decision
                sender.send_output(
                    "[revision] Failed after retries; awaiting your decision...".to_string(),
                );
                // Revision uses planning agent
                let summary = build_workflow_failure_summary(
                    "Revising",
                    &error_msg,
                    Some(&config.workflow.planning.agent),
                    retry_attempts,
                    max_retries,
                    None, // No bundle path for revision failures currently
                );
                sender.send_workflow_failure(summary);

                let decision =
                    wait_for_workflow_failure_decision(&session_logger, approval_rx, control_rx)
                        .await;

                match decision {
                    WorkflowFailureDecision::Retry => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "User chose to retry revision",
                        );
                        retry_attempts = 0; // Reset retry counter for fresh attempt
                        continue;
                    }
                    WorkflowFailureDecision::Stop => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "User chose to stop and save state after revision failure",
                        );
                        // Save failure context for later recovery via CQRS command
                        // Uses planning agent for session continuity
                        let failure = FailureContext::new(
                            FailureKind::Unknown(error_msg),
                            PhaseLabel::Revising,
                            Some(config.workflow.planning.agent.clone().into()),
                            0, // retry_count
                            max_retries as u32,
                            crate::domain::types::TimestampUtc::now(),
                            None, // recovery_action
                        );
                        // Dispatch RecordFailure command (events persisted by actor)
                        dispatch_domain_command(
                            &actor_ref,
                            DomainCommand::RecordFailure { failure },
                            &session_logger,
                        )
                        .await;
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                    WorkflowFailureDecision::Abort => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "User chose to abort after revision failure",
                        );
                        // Dispatch UserAborted command
                        let reason = format!("Revision failed: {}", error_msg);
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
                    WorkflowFailureDecision::Stopped => {
                        session_logger.log(
                            LogLevel::Info,
                            LogCategory::Workflow,
                            "Workflow stopped during revision failure decision",
                        );
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                }
            }
        }
    }

    last_reviews.clear();
    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        "run_revision_phase_with_context completed",
    );

    // Log sequential review state if present (actual reset is done by RevisionCompleted event)
    if let Some(ReviewMode::Sequential(ref seq_state)) = view.review_mode() {
        // Plan version will be incremented by RevisionCompleted event
        let next_version = seq_state.plan_version() + 1;
        sender.send_output(format!(
            "[sequential] Plan revised - restarting from first reviewer (version {})",
            next_version
        ));
        session_logger.log(
            LogLevel::Info,
            LogCategory::Workflow,
            &format!(
                "Sequential review: will reset to first reviewer, plan version {}",
                next_version
            ),
        );
    }

    // Spawn summary generation for the revision phase
    let revision_phase_name = format!("Revising #{}", iteration);
    phases::spawn_summary_generation(
        revision_phase_name,
        view,
        working_dir,
        config,
        sender.clone(),
        None,
        session_logger.clone(),
    );

    // The next iteration will be current + 1 (incremented by RevisionCompleted event)
    let next_iteration = iteration + 1;
    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        &format!(
            "Transitioning: Revising -> Reviewing (iteration will be {})",
            next_iteration
        ),
    );

    // Dispatch RevisionCompleted command to persist phase transition
    // Plan path must exist during revision phase (set at workflow creation)
    let plan_path = view
        .plan_path()
        .cloned()
        .expect("plan_path must be set during Revising phase");
    dispatch_domain_command(
        &actor_ref,
        DomainCommand::RevisionCompleted { plan_path },
        &session_logger,
    )
    .await;
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    Ok(None)
}
