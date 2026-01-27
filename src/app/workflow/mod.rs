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

use crate::domain::actor::WorkflowMessage;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use ractor::ActorRef;
use tokio::sync::oneshot;

/// Dispatches a domain command to the workflow actor with full error handling.
///
/// Handles all three response cases:
/// - `Ok(Ok(_view))` - command succeeded, logs at Info level
/// - `Ok(Err(e))` - command was rejected by actor, logs at Warn level
/// - `Err(_)` - reply channel dropped, logs at Warn level
pub(crate) async fn dispatch_domain_command(
    actor_ref: &Option<ActorRef<WorkflowMessage>>,
    cmd: DomainCommand,
    session_logger: &SessionLogger,
) {
    if let Some(ref actor) = actor_ref {
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(e) =
            actor.send_message(WorkflowMessage::Command(Box::new(cmd.clone()), reply_tx))
        {
            session_logger.log(
                LogLevel::Warn,
                LogCategory::Workflow,
                &format!("Failed to dispatch command {:?}: {}", cmd, e),
            );
            return;
        }
        match reply_rx.await {
            Ok(Ok(_view)) => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    &format!("Command dispatched: {:?}", cmd),
                );
            }
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Command rejected: {:?}: {:?}", cmd, e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Command reply channel dropped: {:?}", cmd),
                );
            }
        }
    }
}

use crate::app::implementation::{run_implementation_workflow, ImplementationContext};
use crate::app::workflow_decisions::{
    await_max_iterations_decision, IterativePhase, MaxIterationsDecision,
};
use crate::config::WorkflowConfig;
use crate::domain::actor::{create_actor_args, WorkflowActor};
use crate::domain::input::WorkflowInput;
use crate::domain::types::{FeedbackPath, Iteration, Phase, PlanPath, WorkingDir};
use crate::planning_paths;
use crate::session_daemon::create_session_logger;
use crate::session_daemon::SessionTracker;
use crate::structured_logger::StructuredLogger;
use crate::tui::{
    CancellationError, Event, SessionEventSender, UserApprovalResponse, WorkflowCommand,
};
use anyhow::Result;
use ractor::Actor;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use completion::handle_completion;
use planning::run_planning_phase;
use reviewing::{
    build_max_iterations_summary_from_view, run_reviewing_phase, run_sequential_reviewing_phase,
    WorkflowPhaseContext,
};
use revising::run_revising_phase;

pub enum WorkflowResult {
    Accepted,
    /// User requested implementation workflow
    ImplementationRequested,
    NeedsRestart {
        user_feedback: String,
    },
    Aborted {
        reason: String,
    },
    /// Workflow was cleanly stopped at a phase boundary
    Stopped,
}

pub struct WorkflowRunConfig {
    pub working_dir: PathBuf,
    pub config: WorkflowConfig,
    pub output_tx: mpsc::UnboundedSender<Event>,
    pub approval_rx: mpsc::Receiver<UserApprovalResponse>,
    pub control_rx: mpsc::Receiver<WorkflowCommand>,
    pub session_id: usize,
    pub run_id: u64,
    /// If true, disable session daemon tracking (for tests/headless mode)
    pub no_daemon: bool,
}

pub async fn run_workflow_with_config(
    input: WorkflowInput,
    run_config: WorkflowRunConfig,
) -> Result<WorkflowResult> {
    let WorkflowRunConfig {
        working_dir,
        config,
        output_tx,
        mut approval_rx,
        mut control_rx,
        session_id,
        run_id,
        no_daemon,
    } = run_config;

    let sender = SessionEventSender::new(session_id, run_id, output_tx);

    // Get workflow session ID from input
    let workflow_session_id = input.workflow_session_id();
    let workflow_session_id_str = workflow_session_id.to_string();

    // Create session logger for workflow events
    let session_logger = create_session_logger(&workflow_session_id_str)?;
    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        "Session logger initialized",
    );

    // Create structured JSONL logger for debugging
    let structured_logger = {
        let logs_dir = planning_paths::session_logs_dir(&workflow_session_id_str)?;
        Arc::new(StructuredLogger::new(&workflow_session_id_str, &logs_dir)?)
    };
    structured_logger.log_workflow_spawn(false);

    // Initialize CQRS actor for event-sourced state management
    let session_dir = planning_paths::session_dir(&workflow_session_id_str)?;
    let (actor_args, view_rx, event_rx) = create_actor_args(&workflow_session_id_str)?;
    let (actor_ref, _actor_handle) = WorkflowActor::spawn(None, WorkflowActor, actor_args)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn workflow actor: {}", e))?;

    // Keep view_rx for reading current view in the main loop
    // Also spawn a task to forward CQRS view updates to TUI
    let view_rx_for_loop = view_rx.clone();
    let view_sender = sender.clone();
    tokio::spawn(async move {
        let mut view_rx = view_rx;
        while view_rx.changed().await.is_ok() {
            let view = view_rx.borrow().clone();
            view_sender.send_view_update(view);
        }
    });

    // Create session tracker for daemon integration early so we can use it for event streaming
    let tracker = Arc::new(SessionTracker::new(no_daemon).await);

    // Spawn task to forward CQRS events to daemon for broadcasting to subscribers
    {
        let session_id_for_events = workflow_session_id_str.clone();
        let tracker_clone = tracker.clone();
        let mut event_rx = event_rx;
        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                // Forward event to daemon (ignore errors - daemon may not be running)
                let _ = tracker_clone
                    .workflow_event(&session_id_for_events, event)
                    .await;
            }
        });
    }

    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        "CQRS workflow actor initialized",
    );

    // For new workflows, send CreateWorkflow command
    // For resumed workflows, the aggregate state is replayed from the event log
    if let WorkflowInput::New(ref new_input) = input {
        let plan_path = planning_paths::session_plan_path(&workflow_session_id_str)?;
        let feedback_path = planning_paths::session_feedback_path(&workflow_session_id_str, 1)?;

        let create_cmd = DomainCommand::CreateWorkflow {
            feature_name: new_input.feature_name.clone(),
            objective: new_input.objective.clone(),
            working_dir: WorkingDir::from(working_dir.as_path()),
            max_iterations: new_input.max_iterations,
            plan_path: PlanPath::from(plan_path),
            feedback_path: FeedbackPath::from(feedback_path),
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        actor_ref
            .send_message(WorkflowMessage::Command(Box::new(create_cmd), reply_tx))
            .map_err(|e| anyhow::anyhow!("Failed to send CreateWorkflow command: {}", e))?;

        // Wait for command result
        match reply_rx.await {
            Ok(Ok(view)) => {
                session_logger.log(
                    LogLevel::Info,
                    LogCategory::Workflow,
                    &format!(
                        "CreateWorkflow succeeded: phase={:?}",
                        view.planning_phase()
                    ),
                );
            }
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("CreateWorkflow command rejected: {:?}", e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "CreateWorkflow reply channel dropped",
                );
            }
        }

        // Dispatch StartPlanning command after workflow creation
        let start_cmd = DomainCommand::StartPlanning;
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(e) =
            actor_ref.send_message(WorkflowMessage::Command(Box::new(start_cmd), reply_tx))
        {
            session_logger.log(
                LogLevel::Warn,
                LogCategory::Workflow,
                &format!("Failed to send StartPlanning command: {}", e),
            );
        } else if reply_rx.await.is_err() {
            session_logger.log(
                LogLevel::Warn,
                LogCategory::Workflow,
                "StartPlanning reply channel dropped",
            );
        }

        // Dispatch AttachWorktree command if worktree info is present
        if let Some(ref wt_info) = new_input.worktree_info {
            let attach_cmd = DomainCommand::AttachWorktree {
                worktree_state: wt_info.clone(),
            };
            let (reply_tx, reply_rx) = oneshot::channel();
            if let Err(e) =
                actor_ref.send_message(WorkflowMessage::Command(Box::new(attach_cmd), reply_tx))
            {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Failed to send AttachWorktree command: {}", e),
                );
            } else if reply_rx.await.is_err() {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "AttachWorktree reply channel dropped",
                );
            }
        }
    }

    // Get the initial view from the actor
    let view = view_rx_for_loop.borrow().clone();

    // Extract feature_name for daemon registration (from input for new, from view for resume)
    let feature_name_for_daemon = match &input {
        WorkflowInput::New(new_input) => new_input.feature_name.0.clone(),
        WorkflowInput::Resume(_) => view.feature_name().map(|f| f.0.clone()).unwrap_or_default(),
    };
    let initial_phase = view.planning_phase().unwrap_or(Phase::Planning);
    let initial_iteration = view.iteration().unwrap_or(Iteration::first()).0;

    // Register session with daemon (now passing session_dir instead of state_path)
    if let Err(e) = tracker
        .register(
            workflow_session_id_str.clone(),
            feature_name_for_daemon,
            working_dir.clone(),
            session_dir.clone(),
            format!("{:?}", initial_phase),
            initial_iteration,
            format!("{:?}", initial_phase),
        )
        .await
    {
        session_logger.log(
            LogLevel::Warn,
            LogCategory::Workflow,
            &format!("Session registration failed (non-fatal): {}", e),
        );
    }

    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        &format!(
            "=== WORKFLOW START: phase={:?}, iteration={} ===",
            initial_phase, initial_iteration
        ),
    );

    let phase_context = WorkflowPhaseContext {
        working_dir: &working_dir,
        config: &config,
        sender: &sender,
        session_logger: session_logger.clone(),
        actor_ref: Some(actor_ref),
    };

    let mut last_reviews: Vec<crate::phases::ReviewResult> = Vec::new();

    loop {
        // Get the current view at the start of each loop iteration
        let view = view_rx_for_loop.borrow().clone();

        if !view.should_continue() {
            break;
        }

        let current_phase = view.planning_phase().unwrap_or(Phase::Planning);
        match current_phase {
            Phase::Planning => {
                let result = run_planning_phase(
                    &view,
                    &working_dir,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                    session_logger.clone(),
                    phase_context.actor_ref.clone(),
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => {
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - PlanningCompleted already dispatched by run_planning_phase
                        // Get updated view for tracker
                        let updated_view = view_rx_for_loop.borrow().clone();
                        let updated_phase =
                            updated_view.planning_phase().unwrap_or(Phase::Reviewing);
                        let updated_iteration =
                            updated_view.iteration().unwrap_or(Iteration::first()).0;

                        // Update session state (daemon tracking is best-effort)
                        let _ = tracker
                            .update(
                                &workflow_session_id_str,
                                format!("{:?}", updated_phase),
                                updated_iteration,
                                format!("{:?}", updated_phase),
                            )
                            .await;
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        // Phase was cancelled - check for command
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Planning phase cancelled, restarting with feedback",
                                    );
                                    return Ok(WorkflowResult::NeedsRestart {
                                        user_feedback: feedback,
                                    });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Planning phase cancelled for stop",
                                    );
                                    // Daemon tracking is best-effort - ignore errors if daemon not running
                                    let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Cancellation without command - shouldn't happen, but treat as abort
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
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
                        &view,
                        &phase_context,
                        &mut approval_rx,
                        &mut control_rx,
                        &mut last_reviews,
                    )
                    .await
                } else {
                    run_reviewing_phase(
                        &view,
                        &phase_context,
                        &mut approval_rx,
                        &mut control_rx,
                        &mut last_reviews,
                    )
                    .await
                };

                match result {
                    Ok(Some(workflow_result)) => {
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - get updated view for tracker
                        let updated_view = view_rx_for_loop.borrow().clone();
                        let updated_phase =
                            updated_view.planning_phase().unwrap_or(Phase::Complete);
                        let updated_iteration =
                            updated_view.iteration().unwrap_or(Iteration::first()).0;

                        // Update session state (daemon tracking is best-effort)
                        let _ = tracker
                            .update(
                                &workflow_session_id_str,
                                format!("{:?}", updated_phase),
                                updated_iteration,
                                format!("{:?}", updated_phase),
                            )
                            .await;
                        if updated_phase == Phase::Complete {
                            break;
                        }
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Reviewing phase cancelled, restarting with feedback",
                                    );
                                    return Ok(WorkflowResult::NeedsRestart {
                                        user_feedback: feedback,
                                    });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Reviewing phase cancelled for stop",
                                    );
                                    // Daemon tracking is best-effort - ignore errors if daemon not running
                                    let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::Revising => {
                // On session resume, last_reviews may be empty - populate from view
                revising::populate_reviews_from_view(&view, &mut last_reviews, &session_logger);

                let result = run_revising_phase(
                    &view,
                    &working_dir,
                    &config,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                    &mut last_reviews,
                    session_logger.clone(),
                    phase_context.actor_ref.clone(),
                )
                .await;

                match result {
                    Ok(Some(workflow_result)) => {
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(workflow_result);
                    }
                    Ok(None) => {
                        // Phase completed - RevisionCompleted dispatched by run_revising_phase
                        // Get updated view for tracker
                        let updated_view = view_rx_for_loop.borrow().clone();
                        let updated_phase =
                            updated_view.planning_phase().unwrap_or(Phase::Reviewing);
                        let updated_iteration =
                            updated_view.iteration().unwrap_or(Iteration::first()).0;

                        // Update session state (daemon tracking is best-effort)
                        let _ = tracker
                            .update(
                                &workflow_session_id_str,
                                format!("{:?}", updated_phase),
                                updated_iteration,
                                format!("{:?}", updated_phase),
                            )
                            .await;
                    }
                    Err(e) if e.downcast_ref::<CancellationError>().is_some() => {
                        if let Ok(cmd) = control_rx.try_recv() {
                            match cmd {
                                WorkflowCommand::Interrupt { feedback } => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Revising phase cancelled, restarting with feedback",
                                    );
                                    return Ok(WorkflowResult::NeedsRestart {
                                        user_feedback: feedback,
                                    });
                                }
                                WorkflowCommand::Stop => {
                                    session_logger.log(
                                        LogLevel::Info,
                                        LogCategory::Workflow,
                                        "Revising phase cancelled for stop",
                                    );
                                    // Daemon tracking is best-effort - ignore errors if daemon not running
                                    let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                                    return Ok(WorkflowResult::Stopped);
                                }
                            }
                        }
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(WorkflowResult::Aborted {
                            reason: "Cancelled without feedback".to_string(),
                        });
                    }
                    Err(e) => return Err(e),
                }
            }

            Phase::AwaitingPlanningDecision => {
                // Re-display the max iterations modal on resume
                let summary = build_max_iterations_summary_from_view(&view, &last_reviews);

                let decision = await_max_iterations_decision(
                    IterativePhase::Planning,
                    &session_logger,
                    &sender,
                    &mut approval_rx,
                    &mut control_rx,
                    summary,
                )
                .await?;

                // Apply the decision using command dispatch
                match decision {
                    MaxIterationsDecision::ProceedWithoutApproval => {
                        phase_context
                            .dispatch_command(DomainCommand::UserOverrideApproval {
                                override_reason:
                                    "User proceeded without AI approval at max iterations"
                                        .to_string(),
                            })
                            .await;
                        sender.send_output(
                            "[planning] Proceeding without AI approval...".to_string(),
                        );
                        // Transition handled by command - loop will pick up Phase::Complete
                    }
                    MaxIterationsDecision::Continue(additional) => {
                        // Extend max_iterations and continue to revising
                        phase_context
                            .dispatch_command(DomainCommand::RevisingStarted {
                                feedback_summary: String::new(),
                                additional_iterations: Some(additional),
                            })
                            .await;
                        sender.send_output(
                            "[planning] Continuing with another review cycle...".to_string(),
                        );
                    }
                    MaxIterationsDecision::RestartWithFeedback(feedback) => {
                        sender.send_output(format!(
                            "[planning] Restarting with feedback: {}",
                            feedback
                        ));
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(WorkflowResult::NeedsRestart {
                            user_feedback: feedback,
                        });
                    }
                    MaxIterationsDecision::Abort => {
                        phase_context
                            .dispatch_command(DomainCommand::UserAborted {
                                reason: "User aborted workflow at max iterations".to_string(),
                            })
                            .await;
                        sender.send_output("[planning] Workflow aborted by user".to_string());
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(WorkflowResult::Aborted {
                            reason: "User aborted workflow at max iterations".to_string(),
                        });
                    }
                    MaxIterationsDecision::Stopped => {
                        // Daemon tracking is best-effort - ignore errors if daemon not running
                        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
                        return Ok(WorkflowResult::Stopped);
                    }
                }
            }

            Phase::Complete => {
                break;
            }
        }
    }

    // Get final view for logging
    let final_view = view_rx_for_loop.borrow().clone();
    let final_phase = final_view.planning_phase().unwrap_or(Phase::Complete);
    let final_iteration = final_view.iteration().unwrap_or(Iteration::first()).0;

    session_logger.log(
        LogLevel::Info,
        LogCategory::Workflow,
        &format!(
            "=== WORKFLOW END: phase={:?}, iteration={} ===",
            final_phase, final_iteration
        ),
    );

    if final_phase == Phase::Complete {
        let result = handle_completion(
            &final_view,
            &session_logger,
            &sender,
            &mut approval_rx,
            &mut control_rx,
        )
        .await?;

        // Dispatch domain command based on user decision
        match &result {
            WorkflowResult::Accepted => {
                phase_context
                    .dispatch_command(DomainCommand::UserApproved)
                    .await;
            }
            WorkflowResult::ImplementationRequested => {
                phase_context
                    .dispatch_command(DomainCommand::UserRequestedImplementation)
                    .await;
            }
            WorkflowResult::NeedsRestart { user_feedback } => {
                phase_context
                    .dispatch_command(DomainCommand::UserDeclined {
                        feedback: user_feedback.clone(),
                    })
                    .await;
            }
            _ => {}
        }

        // Check if implementation was requested
        if matches!(result, WorkflowResult::ImplementationRequested) {
            session_logger.log(
                LogLevel::Info,
                LogCategory::Workflow,
                "Starting implementation workflow",
            );

            let impl_ctx = ImplementationContext {
                session_sender: sender.clone(),
                session_logger: session_logger.clone(),
                approval_rx: &mut approval_rx,
                control_rx: &mut control_rx,
                actor_ref: phase_context.actor_ref.clone(),
            };
            let impl_result = run_implementation_workflow(
                &final_view,
                &config,
                &working_dir,
                impl_ctx,
                None, // No initial feedback
            )
            .await;

            // Mark session stopped after implementation
            // Daemon tracking is best-effort - ignore errors if daemon not running
            let _ = tracker.mark_stopped(&workflow_session_id_str).await;

            match impl_result {
                Ok(impl_outcome) => {
                    use crate::app::implementation::ImplementationWorkflowResult;
                    match impl_outcome {
                        ImplementationWorkflowResult::Approved => {
                            sender.send_output(
                                "[implementation] Implementation complete and approved!"
                                    .to_string(),
                            );
                            return Ok(WorkflowResult::Accepted);
                        }
                        ImplementationWorkflowResult::ApprovedOverridden { iterations_used } => {
                            sender.send_output(format!(
                                "[implementation] Implementation accepted by user override after {} iterations",
                                iterations_used
                            ));
                            return Ok(WorkflowResult::Accepted);
                        }
                        ImplementationWorkflowResult::Failed {
                            iterations_used,
                            last_feedback,
                        } => {
                            let msg = format!(
                                "Implementation failed after {} iterations. Last feedback: {}",
                                iterations_used,
                                last_feedback.as_deref().unwrap_or("none")
                            );
                            sender.send_output(format!("[implementation] {}", msg));
                            return Ok(WorkflowResult::Aborted { reason: msg });
                        }
                        ImplementationWorkflowResult::Cancelled { iterations_used } => {
                            sender.send_output(format!(
                                "[implementation] Cancelled after {} iterations",
                                iterations_used
                            ));
                            return Ok(WorkflowResult::Stopped);
                        }
                        ImplementationWorkflowResult::NoChanges { iterations_used } => {
                            let msg =
                                format!("No changes detected after {} iterations", iterations_used);
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
        // Daemon tracking is best-effort - ignore errors if daemon not running
        let _ = tracker.mark_stopped(&workflow_session_id_str).await;
        return Ok(result);
    }

    sender.send_output("".to_string());
    sender.send_output("=== WORKFLOW COMPLETE ===".to_string());
    sender.send_output("Max iterations reached. Manual review recommended.".to_string());

    // Mark session stopped (daemon tracking is best-effort)
    let _ = tracker.mark_stopped(&workflow_session_id_str).await;
    Ok(WorkflowResult::Accepted)
}
