//! Revising phase execution.

use super::WorkflowResult;
use crate::app::failure::{FailureContext, FailureKind};
use crate::app::util::{build_workflow_failure_summary, log_workflow};
use crate::app::workflow_decisions::{wait_for_workflow_failure_decision, WorkflowFailureDecision};
use crate::config::WorkflowConfig;
use crate::phases::{self, run_revision_phase_with_context};
use crate::state::{Phase, State};
use crate::tui::{CancellationError, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

pub async fn run_revising_phase(
    state: &mut State,
    working_dir: &Path,
    state_path: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    approval_rx: &mut mpsc::Receiver<UserApprovalResponse>,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<Option<WorkflowResult>> {
    // Check for commands before starting revision
    if let Ok(cmd) = control_rx.try_recv() {
        match cmd {
            WorkflowCommand::Interrupt { feedback } => {
                log_workflow(working_dir, &format!("Received interrupt during revising: {}", feedback));
                sender.send_output("[revision] Interrupted by user".to_string());
                return Err(CancellationError { feedback }.into());
            }
            WorkflowCommand::Stop => {
                log_workflow(working_dir, "Received stop during revising");
                sender.send_output("[revision] Stopping...".to_string());
                return Ok(Some(WorkflowResult::Stopped));
            }
        }
    }

    log_workflow(
        working_dir,
        &format!(
            ">>> ENTERING Revising phase (iteration {})",
            state.iteration
        ),
    );
    sender.send_phase_started("Revising".to_string());
    sender.send_output("".to_string());
    sender.send_output(format!(
        "=== REVISION PHASE (Iteration {}) ===",
        state.iteration
    ));
    sender.send_output(format!("Agent: {}", config.workflow.revising.agent));

    let max_retries = config.failure_policy.max_retries as usize;
    let mut retry_attempts = 0usize;

    loop {
        log_workflow(working_dir, "Calling run_revision_phase_with_context...");
        let revision_result = run_revision_phase_with_context(
            state,
            working_dir,
            config,
            last_reviews,
            sender.clone(),
            state.iteration,
            state_path,
        )
        .await;

        match revision_result {
            Ok(()) => {
                // Success - continue with the rest of the phase
                break;
            }
            Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                log_workflow(working_dir, "Revising phase was cancelled");
                return Err(e);
            }
            Err(e) => {
                let error_msg = format!("{}", e);
                log_workflow(working_dir, &format!("Revising phase failed: {}", error_msg));
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
                sender.send_output("[revision] Failed after retries; awaiting your decision...".to_string());
                let summary = build_workflow_failure_summary(
                    "Revising",
                    &error_msg,
                    Some(&config.workflow.revising.agent),
                    retry_attempts,
                    max_retries,
                    None, // No bundle path for revision failures currently
                );
                sender.send_workflow_failure(summary);

                let decision = wait_for_workflow_failure_decision(working_dir, approval_rx, control_rx).await;

                match decision {
                    WorkflowFailureDecision::Retry => {
                        log_workflow(working_dir, "User chose to retry revision");
                        retry_attempts = 0; // Reset retry counter for fresh attempt
                        continue;
                    }
                    WorkflowFailureDecision::Stop => {
                        log_workflow(working_dir, "User chose to stop and save state after revision failure");
                        // Save failure context for later recovery
                        state.set_failure(FailureContext {
                            kind: FailureKind::Unknown(error_msg),
                            phase: state.phase.clone(),
                            agent_name: Some(config.workflow.revising.agent.clone()),
                            retry_count: retry_attempts as u32,
                            max_retries: max_retries as u32,
                            failed_at: chrono::Utc::now().to_rfc3339(),
                            recovery_action: None,
                        });
                        state.set_updated_at();
                        state.save_atomic(state_path)?;
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                    WorkflowFailureDecision::Abort => {
                        log_workflow(working_dir, "User chose to abort after revision failure");
                        return Ok(Some(WorkflowResult::Aborted {
                            reason: format!("Revision failed: {}", error_msg),
                        }));
                    }
                    WorkflowFailureDecision::Stopped => {
                        log_workflow(working_dir, "Workflow stopped during revision failure decision");
                        return Ok(Some(WorkflowResult::Stopped));
                    }
                }
            }
        }
    }

    last_reviews.clear();
    log_workflow(working_dir, "run_revision_phase_with_context completed");

    // Keep old feedback files - don't cleanup
    // let feedback_path = working_dir.join(&state.feedback_file);
    // match cleanup_merged_feedback(&feedback_path) { ... }

    let revision_phase_name = format!("Revising #{}", state.iteration);
    phases::spawn_summary_generation(
        revision_phase_name,
        state,
        working_dir,
        config,
        sender.clone(),
        None,
    );

    state.iteration += 1;
    // Update feedback filename for the new iteration before transitioning to review
    state.update_feedback_for_iteration(state.iteration);
    log_workflow(
        working_dir,
        &format!(
            "Transitioning: Revising -> Reviewing (iteration now {})",
            state.iteration
        ),
    );
    state.transition(Phase::Reviewing)?;
    state.set_updated_at();
    state.save_atomic(state_path)?;
    sender.send_state_update(state.clone());
    sender.send_output("[planning] Transitioning to review phase...".to_string());

    Ok(None)
}
