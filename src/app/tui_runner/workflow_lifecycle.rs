//! Workflow lifecycle management: initialization completion and workflow state transitions.
//!
//! This module handles the lifecycle events of workflows including:
//! - Initialization completion (resuming sessions, starting new workflows)
//! - Workflow completion handling (success, abort, restart, stop)

use crate::app::util::build_resume_command;
use crate::app::workflow::{WorkflowResult, WorkflowRunConfig};
use crate::config::WorkflowConfig;
use crate::domain::{WorkflowInput, WorkflowView};
use crate::tui::session::context::compute_effective_working_dir;
use crate::tui::{Session, SessionStatus, TabManager, UserApprovalResponse, WorkflowCommand};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use super::{run_workflow_with_config, ResumableSession};

/// Starts a workflow for a resumed session.
///
/// This helper sets up the workflow channels and spawns the workflow task.
/// Used by both CLI --resume and /sessions overlay resume.
///
/// **Note on workflow_config parameter:** Callers are responsible for providing
/// the correct workflow_config based on their context:
/// - `--resume` CLI: Use `load_workflow_config(cli, ...)` to respect CLI flags
/// - `/sessions` browser: Use `load_workflow_from_selection(...)` to respect current selection
///
/// The function uses session context if available:
/// - `session.context.effective_working_dir` for workflow execution (worktree-aware)
/// - Falls back to `working_dir` parameter if context is not set
pub fn start_resumed_workflow(
    session: &mut Session,
    input: WorkflowInput,
    view: WorkflowView,
    working_dir: &Path,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) {
    session.workflow_view = Some(view.clone());
    session.status = SessionStatus::Planning;
    session.running = true;

    let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
    session.approval_tx = Some(new_approval_tx);

    let (new_control_tx, new_control_rx) = mpsc::channel::<WorkflowCommand>(1);
    session.workflow_control_tx = Some(new_control_tx);

    session.current_run_id += 1;
    let run_id = session.current_run_id;

    // Use effective_working_dir from session context if available,
    // otherwise compute from view's worktree_info or fall back to working_dir
    let effective_working_dir = session
        .context
        .as_ref()
        .map(|ctx| ctx.effective_working_dir.clone())
        .unwrap_or_else(|| compute_effective_working_dir(working_dir, view.worktree_info()));

    let cfg = workflow_config.clone();
    let workflow_handle = tokio::spawn({
        let working_dir = effective_working_dir;
        let tx = output_tx.clone();
        let sid = session.id;
        async move {
            run_workflow_with_config(
                input,
                WorkflowRunConfig {
                    working_dir,
                    config: cfg,
                    output_tx: tx,
                    approval_rx: new_approval_rx,
                    control_rx: new_control_rx,
                    session_id: sid,
                    run_id,
                    no_daemon: false,
                },
            )
            .await
        }
    });

    session.workflow_handle = Some(workflow_handle);
}

/// Result from initialization task containing all data needed to start a workflow.
pub struct InitResult {
    /// The workflow input (New or Resume).
    pub input: WorkflowInput,
    /// Initial workflow view (populated from events for resume, None for new workflows).
    /// For new workflows, the view is projected from WorkflowCreated event through CQRS.
    pub view: Option<WorkflowView>,
    /// Path to store workflow state.
    pub state_path: PathBuf,
    /// Human-readable feature name.
    pub feature_name: String,
    /// Effective working directory (may differ from base if using worktrees).
    pub effective_working_dir: PathBuf,
}

/// Handles the completion of session initialization.
///
/// This is called when an init task (loading state, extracting feature name, etc.)
/// completes. It sets up the workflow channels, session context, and spawns the workflow task.
///
/// Note: Workflow configuration is loaded dynamically from the persisted selection
/// for base_working_dir, ensuring that /workflow selections are always respected.
pub async fn handle_init_completion(
    session_id: usize,
    handle: tokio::task::JoinHandle<anyhow::Result<InitResult>>,
    tab_manager: &mut TabManager,
    base_working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) {
    match handle.await {
        Ok(Ok(init_result)) => {
            let InitResult {
                input,
                view,
                state_path,
                feature_name,
                effective_working_dir,
            } = init_result;

            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                // Load workflow config from persisted selection for this working directory
                // This ensures /workflow changes are respected for new sessions
                let workflow_config =
                    crate::app::tui_runner::workflow_loading::load_workflow_from_selection(
                        base_working_dir,
                    );

                session.name = feature_name;
                // For resume: view is populated from events. For new: None (will be set via CQRS).
                session.workflow_view = view.clone();

                // Set up session context with computed effective_working_dir
                session.context = Some(crate::tui::SessionContext::new(
                    base_working_dir.to_path_buf(),
                    Some(effective_working_dir.clone()),
                    state_path.clone(),
                    workflow_config.clone(),
                ));

                // Check if view has a failure that needs recovery (from stopped sessions)
                // Only applies to resumed sessions - new workflows have view = None
                if let Some(ref v) = view {
                    if let Some(failure) = v.last_failure() {
                        let summary = crate::app::util::build_resume_failure_summary(failure);
                        if matches!(
                            failure.kind(),
                            crate::domain::failure::FailureKind::AllReviewersFailed
                        ) {
                            session.start_all_reviewers_failed(summary);
                        } else {
                            session.start_workflow_failure(summary);
                        }
                        session.add_output(
                            "[planning] Session has unresolved failure - awaiting recovery decision"
                                .to_string(),
                        );
                        return;
                    }
                }

                let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
                session.approval_tx = Some(new_approval_tx);

                // Create control channel for workflow interrupts
                let (new_control_tx, new_control_rx) = mpsc::channel::<WorkflowCommand>(1);
                session.workflow_control_tx = Some(new_control_tx);

                // Increment run_id for this new workflow
                session.current_run_id += 1;
                let run_id = session.current_run_id;

                let cfg = workflow_config.clone();
                let workflow_handle = tokio::spawn({
                    let working_dir = effective_working_dir;
                    let tx = output_tx.clone();
                    let sid = session_id;
                    async move {
                        run_workflow_with_config(
                            input,
                            WorkflowRunConfig {
                                working_dir,
                                config: cfg,
                                output_tx: tx,
                                approval_rx: new_approval_rx,
                                control_rx: new_control_rx,
                                session_id: sid,
                                run_id,
                                no_daemon: false,
                            },
                        )
                        .await
                    }
                });

                session.workflow_handle = Some(workflow_handle);
            }
        }
        Ok(Err(e)) => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_error(&format!("Initialization failed: {}", e));
            }
        }
        Err(e) => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_error(&format!("Initialization panicked: {}", e));
            }
        }
    }
}

/// Checks all sessions for completed workflows and handles their results.
///
/// Returns a list of sessions that were stopped and can be resumed later.
pub async fn check_workflow_completions(
    tab_manager: &mut TabManager,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) -> Vec<ResumableSession> {
    let mut resumable_sessions = Vec::new();

    for session in tab_manager.sessions_mut() {
        if let Some(handle) = session.workflow_handle.take() {
            let handle: tokio::task::JoinHandle<anyhow::Result<WorkflowResult>> = handle;
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(WorkflowResult::Accepted)) => {
                        session.status = SessionStatus::Complete;
                        session.running = false;
                        session.workflow_control_tx = None;
                    }
                    Ok(Ok(WorkflowResult::Aborted { reason })) => {
                        session.status = SessionStatus::Error;
                        session.running = false;
                        session.error_state = Some(reason);
                        session.workflow_control_tx = None;
                    }
                    Ok(Ok(WorkflowResult::NeedsRestart { user_feedback })) => {
                        // handle_workflow_restart now loads config internally
                        handle_workflow_restart(session, &user_feedback, working_dir, output_tx);
                    }
                    Ok(Ok(WorkflowResult::Stopped)) => {
                        if let Some(resumable) = handle_workflow_stopped(session, working_dir) {
                            resumable_sessions.push(resumable);
                        }
                    }
                    Ok(Ok(WorkflowResult::ImplementationRequested)) => {
                        session.status = SessionStatus::Planning;
                    }
                    Ok(Err(e)) => {
                        session.handle_error(&format!("Workflow failed: {}", e));
                    }
                    Err(e) => {
                        session.handle_error(&format!("Workflow panicked: {}", e));
                    }
                }
            } else {
                session.workflow_handle = Some(handle);
            }
        }
    }

    resumable_sessions
}

/// Handles a workflow that needs to restart with user feedback.
///
/// Uses session context for working directory and config if available:
/// - `context.base_working_dir` for state_path computation
/// - `context.effective_working_dir` for workflow execution (worktree-aware)
/// - `context.workflow_config` for workflow configuration
/// - Falls back to loading from persisted selection if context is not set
fn handle_workflow_restart(
    session: &mut crate::tui::Session,
    user_feedback: &str,
    global_working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) {
    session.add_output("".to_string());
    session.add_output("=== RESTARTING WITH YOUR FEEDBACK ===".to_string());
    session.add_output(format!("Changes requested: {}", user_feedback));

    session.streaming_lines.clear();
    session.clear_todos();
    session.run_tabs.clear();
    session.active_run_tab = 0;
    session.chat_follow_mode = true;
    session.status = SessionStatus::Planning;

    // Get working directories and config from session context or load from selection
    let (base_working_dir, effective_working_dir, workflow_config) =
        if let Some(ref ctx) = session.context {
            // Session has context - use stored values
            (
                ctx.base_working_dir.clone(),
                ctx.effective_working_dir.clone(),
                ctx.workflow_config.clone(),
            )
        } else if let Some(ref view) = session.workflow_view {
            // No context but have view - compute effective_working_dir from worktree_info
            // and load workflow config from current selection
            let effective = compute_effective_working_dir(global_working_dir, view.worktree_info());
            let workflow_config =
                crate::app::tui_runner::workflow_loading::load_workflow_from_selection(
                    global_working_dir,
                );
            (global_working_dir.to_path_buf(), effective, workflow_config)
        } else {
            // No context and no view - use global working dir and load from selection
            let workflow_config =
                crate::app::tui_runner::workflow_loading::load_workflow_from_selection(
                    global_working_dir,
                );
            (
                global_working_dir.to_path_buf(),
                global_working_dir.to_path_buf(),
                workflow_config,
            )
        };

    // Validate effective_working_dir still exists (worktree may have been deleted)
    let effective_working_dir = if effective_working_dir.exists() {
        effective_working_dir
    } else {
        session.add_output(format!(
            "[planning] Warning: Worktree path no longer exists: {}",
            effective_working_dir.display()
        ));
        session.add_output(format!(
            "[planning] Falling back to base directory: {}",
            base_working_dir.display()
        ));
        base_working_dir.clone()
    };

    // Get workflow_id and feature_name from view for resume
    let Some(ref view) = session.workflow_view else {
        session.handle_error("No workflow view available for restart");
        return;
    };

    let Some(workflow_id) = view.workflow_id() else {
        session.handle_error("No workflow ID available for restart");
        return;
    };

    let feature_name = view.feature_name().map(|f| f.0.clone()).unwrap_or_default();

    // Use base_working_dir for state_path computation (consistent with how sessions are stored)
    let _state_path = session
        .context
        .as_ref()
        .map(|ctx| ctx.state_path.clone())
        .or_else(|| crate::planning_paths::state_path(&base_working_dir, &feature_name).ok());

    let Some(_state_path) = _state_path else {
        session.handle_error("Failed to get state path");
        return;
    };

    // Create resume input - the workflow engine will handle the restart logic
    let input = WorkflowInput::Resume(crate::domain::ResumeWorkflowInput {
        workflow_id: workflow_id.clone(),
    });

    let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
    session.approval_tx = Some(new_approval_tx);

    let (new_control_tx, new_control_rx) = mpsc::channel::<WorkflowCommand>(1);
    session.workflow_control_tx = Some(new_control_tx);

    session.current_run_id += 1;
    let run_id = session.current_run_id;

    let cfg = workflow_config;
    let new_handle = tokio::spawn({
        let working_dir = effective_working_dir;
        let tx = output_tx.clone();
        let sid = session.id;
        async move {
            run_workflow_with_config(
                input,
                WorkflowRunConfig {
                    working_dir,
                    config: cfg,
                    output_tx: tx,
                    approval_rx: new_approval_rx,
                    control_rx: new_control_rx,
                    session_id: sid,
                    run_id,
                    no_daemon: false,
                },
            )
            .await
        }
    });

    session.workflow_handle = Some(new_handle);
}

/// Handles a workflow that was stopped by the user.
/// Returns a ResumableSession if the snapshot was saved successfully.
///
/// Uses session context for working_dir if available:
/// - `context.base_working_dir` for snapshot saving and resume command
/// - Falls back to global `working_dir` if context is not set
fn handle_workflow_stopped(
    session: &mut crate::tui::Session,
    global_working_dir: &Path,
) -> Option<ResumableSession> {
    // Use base_working_dir from session context or fall back to global
    let base_working_dir = session
        .context
        .as_ref()
        .map(|ctx| ctx.base_working_dir.clone())
        .unwrap_or_else(|| global_working_dir.to_path_buf());

    // Note: Snapshot saving with the new CQRS model uses the event store,
    // so we don't need to save explicit state here. The event log is the source of truth.

    session.status = SessionStatus::Stopped;
    session.running = false;
    session.workflow_control_tx = None;
    session.add_output("".to_string());
    session.add_output("=== SESSION STOPPED ===".to_string());

    // Extract session info from workflow_view
    let session_info = session.workflow_view.as_ref().and_then(|view| {
        let workflow_id = view.workflow_id()?.to_string();
        let feature_name = view.feature_name()?.0.clone();
        Some((workflow_id, feature_name))
    });

    if let Some((workflow_session_id, feature_name)) = session_info {
        session.add_output(format!("Session ID: {}", workflow_session_id));
        let resume_cmd = build_resume_command(&workflow_session_id, &base_working_dir);
        session.add_output(format!("To resume: {}", resume_cmd));

        return Some(ResumableSession::new(
            feature_name,
            workflow_session_id,
            base_working_dir,
        ));
    }

    None
}
