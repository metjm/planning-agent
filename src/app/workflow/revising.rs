//! Revising phase execution.

use crate::app::util::log_workflow;
use crate::config::WorkflowConfig;
use crate::phases::{self, run_revision_phase_with_context};
use crate::state::{Phase, State};
use crate::tui::{CancellationError, SessionEventSender, WorkflowCommand};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

pub async fn run_revising_phase(
    state: &mut State,
    working_dir: &Path,
    state_path: &Path,
    config: &WorkflowConfig,
    sender: &SessionEventSender,
    control_rx: &mut mpsc::Receiver<WorkflowCommand>,
    last_reviews: &mut Vec<phases::ReviewResult>,
) -> Result<()> {
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
                // Return a special error that signals stop - will be handled by caller
                return Err(anyhow::anyhow!("Workflow stopped"));
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

    // Check for cancellation
    match revision_result {
        Ok(()) => {}
        Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
            log_workflow(working_dir, "Revising phase was cancelled");
            return Err(e);
        }
        Err(e) => return Err(e),
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

    Ok(())
}
