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

use crate::app::implementation::run_implementation_workflow;
use crate::config::WorkflowConfig;
use crate::planning_paths;
use crate::session_logger::{create_session_logger, LogCategory, LogLevel};
use crate::session_tracking::SessionTracker;
use crate::state::{Phase, State};
use crate::state_machine::{StateSnapshot, WorkflowStateMachine};
use crate::structured_logger::StructuredLogger;
use crate::tui::{CancellationError, Event, SessionEventSender, UserApprovalResponse, WorkflowCommand};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

use completion::handle_completion;
use planning::run_planning_phase;
use reviewing::{run_reviewing_phase, run_sequential_reviewing_phase, WorkflowPhaseContext};
use revising::run_revising_phase;


pub enum WorkflowResult {
    Accepted,
    /// User requested implementation workflow
    ImplementationRequested,
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
    /// If true, disable session daemon tracking (for tests/headless mode)
    #[allow(dead_code)]
    pub no_daemon: bool,
    /// Optional watch channel sender for broadcasting state snapshots to TUI.
    /// When provided, the workflow will broadcast StateSnapshot updates that
    /// the TUI can poll for state changes. This enables the new centralized
    /// state management architecture.
    ///
    /// If None (default), the workflow uses legacy Event::SessionStateUpdate.
    pub snapshot_tx: Option<watch::Sender<StateSnapshot>>,
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
        no_daemon,
        snapshot_tx,
    } = run_config;

    let sender = SessionEventSender::new(session_id, run_id, output_tx);

    // Create session logger for workflow events
    let session_logger = create_session_logger(&state.workflow_session_id)?;
    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Session logger initialized");

    // Create structured JSONL logger for debugging
    let structured_logger = {
        let logs_dir = planning_paths::session_logs_dir(&state.workflow_session_id)?;
        Arc::new(StructuredLogger::new(&state.workflow_session_id, &logs_dir)?)
    };
    structured_logger.log_workflow_spawn(false);

    // Create state machine if snapshot_tx is provided (new architecture)
    // Otherwise fall back to legacy direct state mutation
    let state_machine: Option<WorkflowStateMachine> = if let Some(tx) = snapshot_tx {
        // Create state machine that will broadcast to the provided channel
        // Note: We create a new state machine wrapping the existing state,
        // but we need to use the existing tx from the caller
        let initial_snapshot = StateSnapshot::from(&state);
        let _ = tx.send(initial_snapshot);

        // For now, we create the state machine but continue using direct state mutation
        // TODO: Migrate phase handlers to use state machine commands
        let (machine, _rx) = WorkflowStateMachine::new(state.clone(), structured_logger.clone());

        // Store the tx for broadcasting (but we don't use the machine's rx)
        // The TUI already has the rx from when it called watch::channel()
        drop(_rx);

        // Actually, we need to think about this differently.
        // The state machine creates its own watch channel internally.
        // We should either:
        // 1. Pass the tx into the state machine constructor, or
        // 2. Return the rx from the state machine and use that
        //
        // For incremental migration, let's keep it simple:
        // We'll create the state machine with its own watch channel,
        // and just broadcast snapshots manually after state changes.

        Some(machine)
    } else {
        None
    };

    // Suppress unused warning for now - will be used when phase handlers are migrated
    let _ = state_machine;

    // Create session tracker for daemon integration
    let tracker = Arc::new(SessionTracker::new(no_daemon));

    // Register session with daemon
    if let Err(e) = tracker
        .register(
            state.workflow_session_id.clone(),
            state.feature_name.clone(),
            working_dir.clone(),
            state_path.clone(),
            format!("{:?}", state.phase),
            state.iteration,
            format!("{:?}", state.phase),
        )
        .await
    {
        session_logger.log(LogLevel::Warn, LogCategory::Workflow, &format!("Session registration failed (non-fatal): {}", e));
    }

    session_logger.log(LogLevel::Info, LogCategory::Workflow, &format!(
        "=== WORKFLOW START: phase={:?}, iteration={} ===",
        state.phase, state.iteration
    ));

    let phase_context = WorkflowPhaseContext {
        working_dir: &working_dir,
        state_path: &state_path,
        config: &config,
        sender: &sender,
        session_logger: session_logger.clone(),
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
                    session_logger.clone(),
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => {
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - update session state
                        let _ = tracker
                            .update(
                                &state.workflow_session_id,
                                format!("{:?}", state.phase),
                                state.iteration,
                                format!("{:?}", state.phase),
                            )
                            .await;
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        // Phase was cancelled - check for command
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Planning phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Planning phase cancelled for stop");
                                    let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Cancellation without command - shouldn't happen, but treat as abort
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Reviewing => {
                // Choose sequential or parallel review based on config
                let result = if config.workflow.reviewing.sequential {
                    run_sequential_reviewing_phase(
                        &mut state,
                        &phase_context,
                        &mut approval_rx,
                        &mut control_rx,
                        &mut last_reviews,
                    )
                    .await
                } else {
                    run_reviewing_phase(
                        &mut state,
                        &phase_context,
                        &mut approval_rx,
                        &mut control_rx,
                        &mut last_reviews,
                    )
                    .await
                };

                match result {
                    Ok(Some(workflow_result)) => {
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - update session state
                        let _ = tracker
                            .update(
                                &state.workflow_session_id,
                                format!("{:?}", state.phase),
                                state.iteration,
                                format!("{:?}", state.phase),
                            )
                            .await;
                        if state.phase == Phase::Complete {
                            break;
                        }
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Reviewing phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Reviewing phase cancelled for stop");
                                    let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
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
                    session_logger.clone(),
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => {
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - update session state
                        let _ = tracker
                            .update(
                                &state.workflow_session_id,
                                format!("{:?}", state.phase),
                                state.iteration,
                                format!("{:?}", state.phase),
                            )
                            .await;
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Revising phase cancelled, restarting with feedback");
                                    return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(LogLevel::Info, LogCategory::Workflow, "Revising phase cancelled for stop");
                                    let _ = tracker.mark_stopped(&state.workflow_session_id).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
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

    session_logger.log(LogLevel::Info, LogCategory::Workflow, &format!(
        "=== WORKFLOW END: phase={:?}, iteration={} ===",
        state.phase, state.iteration
    ));

    if state.phase == Phase::Complete {
        let result = handle_completion(
            &state,
            &working_dir,
            &sender,
            &mut approval_rx,
            &mut control_rx,
        )
        .await?;

        // Check if implementation was requested
        if matches!(result, WorkflowResult::ImplementationRequested) {
            session_logger.log(LogLevel::Info, LogCategory::Workflow, "Starting implementation workflow");

            let impl_result = run_implementation_workflow(
                &mut state,
                &config,
                &working_dir,
                sender.clone(),
                session_logger.clone(),
                None, // No initial feedback
            )
            .await;

            // Mark session stopped after implementation
            let _ = tracker.mark_stopped(&state.workflow_session_id).await;

            match impl_result {
                Ok(impl_outcome) => {
                    use crate::app::implementation::ImplementationWorkflowResult;
                    match impl_outcome {
                        ImplementationWorkflowResult::Approved => {
                            sender.send_output("[implementation] Implementation complete and approved!".to_string());
                            return Ok(WorkflowResult::Accepted);
                        }
                        ImplementationWorkflowResult::Failed { iterations_used, last_feedback } => {
                            let msg = format!(
                                "Implementation failed after {} iterations. Last feedback: {}",
                                iterations_used,
                                last_feedback.as_deref().unwrap_or("none")
                            );
                            sender.send_output(format!("[implementation] {}", msg));
                            return Ok(WorkflowResult::Aborted { reason: msg });
                        }
                        ImplementationWorkflowResult::Cancelled { iterations_used } => {
                            sender.send_output(format!("[implementation] Cancelled after {} iterations", iterations_used));
                            return Ok(WorkflowResult::Stopped);
                        }
                        ImplementationWorkflowResult::NoChanges { iterations_used } => {
                            let msg = format!("No changes detected after {} iterations", iterations_used);
                            sender.send_output(format!("[implementation] {}", msg));
                            return Ok(WorkflowResult::Aborted { reason: msg });
                        }
                    }
                }
                Err(e) => {
                    sender.send_output(format!("[implementation] Error: {}", e));
                    return Err(e);
                }
            }
        }

        // Mark session stopped after completion handling
        let _ = tracker.mark_stopped(&state.workflow_session_id).await;
        return Ok(result);
    }

    sender.send_output("".to_string());
    sender.send_output("=== WORKFLOW COMPLETE ===".to_string());
    sender.send_output("Max iterations reached. Manual review recommended.".to_string());

    // Mark session stopped
    let _ = tracker.mark_stopped(&state.workflow_session_id).await;
    Ok(WorkflowResult::Accepted)
}
