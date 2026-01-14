//! Workflow execution engine.
//!
//! # Important Pattern: Channel Handling
//!
//! When waiting on channel receives in this module, **ALWAYS use `tokio::select!`**
//! to simultaneously check both `approval_rx` AND `control_rx` channels.
//!
//! The `control_rx` channel receives `WorkflowCommand::Stop` when the user quits
//! the TUI. If a function only awaits on `approval_rx.recv()`, it will block
//! indefinitely when the user quits, causing the "quit timeout reached with
//! active workflows" freeze bug.
//!
//! Good pattern (see `wait_for_review_decision` in workflow_decisions.rs):
//! ```ignore
//! loop {
//!     tokio::select! {
//!         Some(cmd) = control_rx.recv() => {
//!             if matches!(cmd, WorkflowCommand::Stop) {
//!                 return Ok(WorkflowResult::Stopped);
//!             }
//!         }
//!         response = approval_rx.recv() => {
//!             // Handle approval response
//!         }
//!     }
//! }
//! ```
//!
//! Bad pattern (causes freeze):
//! ```ignore
//! loop {
//!     match approval_rx.recv().await {  // WRONG: doesn't check control_rx!
//!         // ...
//!     }
//! }
//! ```

mod completion;
mod planning;
mod reviewing;
mod revising;

use crate::app::util::log_workflow;
use crate::config::WorkflowConfig;
use crate::state::{Phase, State};
use crate::tui::{CancellationError, Event, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use completion::handle_completion;
use planning::run_planning_phase;
use reviewing::{run_reviewing_phase, WorkflowPhaseContext};
use revising::run_revising_phase;

pub enum WorkflowResult {
    Accepted,
    NeedsRestart { user_feedback: String },
    Aborted { reason: String },
    /// Workflow was cleanly stopped at a phase boundary
    Stopped,
}

pub struct WorkflowRunConfig {
    pub working_dir: PathBuf,
    pub state_path: PathBuf,
    pub config: WorkflowConfig,
    pub output_tx: mpsc::UnboundedSender<Event>,
    pub approval_rx: mpsc::Receiver<UserApprovalResponse>,
    pub control_rx: mpsc::Receiver<WorkflowCommand>,
    pub session_id: usize,
    pub run_id: u64,
}

pub async fn run_workflow_with_config(
    mut state: State,
    run_config: WorkflowRunConfig,
) -> Result<WorkflowResult> {
    let WorkflowRunConfig {
        working_dir,
        state_path,
        config,
        output_tx,
        mut approval_rx,
        mut control_rx,
        session_id,
        run_id,
    } = run_config;

    let sender = SessionEventSender::new(session_id, run_id, output_tx);

    log_workflow(
        &working_dir,
        &format!(
            "=== WORKFLOW START: phase={:?}, iteration={} ===",
            state.phase, state.iteration
        ),
    );

    let phase_context = WorkflowPhaseContext {
        working_dir: &working_dir,
        state_path: &state_path,
        config: &config,
        sender: &sender,
    };

    let mut last_reviews: Vec<crate::phases::ReviewResult> = Vec::new();

    loop {
        if !state.should_continue() {
            break;
        }

        match state.phase {
            Phase::Planning => {
                let result = run_planning_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => return Ok(workflow_result),
                    Ok(None) => {}
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        // Phase was cancelled - check for command
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Planning phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Planning phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Cancellation without command - shouldn't happen, but treat as abort
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Reviewing => {
                let result = run_reviewing_phase(
                    &mut state,
                    &phase_context,
                    &mut approval_rx,
                    &mut control_rx,
                    &mut last_reviews,
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => return Ok(workflow_result),
                    Ok(None) => {
                        if state.phase == Phase::Complete {
                            break;
                        }
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Reviewing phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Reviewing phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Revising => {
                let result = run_revising_phase(
                    &mut state,
                    &working_dir,
                    &state_path,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                    &mut last_reviews,
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => return Ok(workflow_result),
                    Ok(None) => {}
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    log_workflow(&working_dir, "Revising phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    log_workflow(&working_dir, "Revising phase cancelled for stop");
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Complete => {
                break;
            }
        }
    }

    log_workflow(
        &working_dir,
        &format!(
            "=== WORKFLOW END: phase={:?}, iteration={} ===",
            state.phase, state.iteration
        ),
    );

    if state.phase == Phase::Complete {
        return handle_completion(
            &state,
            &working_dir,
            &sender,
            &mut approval_rx,
            &mut control_rx,
        )
        .await;
    }

    sender.send_output("".to_string());
    sender.send_output("=== WORKFLOW COMPLETE ===".to_string());
    sender.send_output("Max iterations reached. Manual review recommended.".to_string());

    Ok(WorkflowResult::Accepted)
}
