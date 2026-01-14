//! Workflow lifecycle management: initialization completion and workflow state transitions.
//!
//! This module handles the lifecycle events of workflows including:
//! - Initialization completion (resuming sessions, starting new workflows)
//! - Workflow completion handling (success, abort, restart, stop)

use crate::app::util::build_resume_command;
use crate::app::workflow::{WorkflowResult, WorkflowRunConfig};
use crate::config::WorkflowConfig;
use crate::planning_paths;
use crate::state::{Phase, State};
use crate::tui::{Session, SessionStatus, TabManager, UserApprovalResponse, WorkflowCommand};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use super::snapshot_helper::create_and_save_snapshot;
use super::{run_workflow_with_config, ResumableSession};

/// Starts a workflow for a resumed session.
///
/// This helper sets up the workflow channels and spawns the workflow task.
/// Used by both CLI --resume and /sessions overlay resume.
pub fn start_resumed_workflow(
    session: &mut Session,
    state: State,
    state_path: PathBuf,
    working_dir: &Path,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) {
    session.workflow_state = Some(state.clone());
    session.status = SessionStatus::Planning;
    session.running = true;

    let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
    session.approval_tx = Some(new_approval_tx);

    let (new_control_tx, new_control_rx) = mpsc::channel::<WorkflowCommand>(1);
    session.workflow_control_tx = Some(new_control_tx);

    session.current_run_id += 1;
    let run_id = session.current_run_id;

    let cfg = workflow_config.clone();
    let workflow_handle = tokio::spawn({
        let working_dir = working_dir.to_path_buf();
        let tx = output_tx.clone();
        let sid = session.id;
        async move {
            run_workflow_with_config(
                state,
                WorkflowRunConfig {
                    working_dir,
                    state_path,
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

/// Handles the completion of session initialization.
///
/// This is called when an init task (loading state, extracting feature name, etc.)
/// completes. It sets up the workflow channels and spawns the workflow task.
pub async fn handle_init_completion(
    session_id: usize,
    handle: tokio::task::JoinHandle<anyhow::Result<(State, PathBuf, String)>>,
    tab_manager: &mut TabManager,
    working_dir: &Path,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<crate::tui::Event>,
) {
    match handle.await {
        Ok(Ok((state, state_path, feature_name))) => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.name = feature_name;
                session.workflow_state = Some(state.clone());

                // Check if state has a failure that needs recovery (from stopped sessions)
                if let Some(ref failure) = state.last_failure {
                    let summary = crate::app::util::build_resume_failure_summary(failure);
                    if matches!(failure.kind, crate::app::failure::FailureKind::AllReviewersFailed) {
                        session.start_all_reviewers_failed(summary);
                    } else {
                        session.start_workflow_failure(summary);
                    }
                    session.add_output("[planning] Session has unresolved failure - awaiting recovery decision".to_string());
                    return;
                }

                let (new_approval_tx, new_approval_rx) =
                    mpsc::channel::<UserApprovalResponse>(1);
                session.approval_tx = Some(new_approval_tx);

                // Create control channel for workflow interrupts
                let (new_control_tx, new_control_rx) =
                    mpsc::channel::<WorkflowCommand>(1);
                session.workflow_control_tx = Some(new_control_tx);

                // Increment run_id for this new workflow
                session.current_run_id += 1;
                let run_id = session.current_run_id;

                let cfg = workflow_config.clone();
                let workflow_handle = tokio::spawn({
                    let working_dir = working_dir.to_path_buf();
                    let tx = output_tx.clone();
                    let sid = session_id;
                    async move {
                        run_workflow_with_config(
                            state,
                            WorkflowRunConfig {
                                working_dir,
                                state_path,
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
    workflow_config: &WorkflowConfig,
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
                        handle_workflow_restart(session, &user_feedback, working_dir, workflow_config, output_tx);
                    }
                    Ok(Ok(WorkflowResult::Stopped)) => {
                        if let Some(resumable) = handle_workflow_stopped(session, working_dir) {
                            resumable_sessions.push(resumable);
                        }
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
fn handle_workflow_restart(
    session: &mut crate::tui::Session,
    user_feedback: &str,
    working_dir: &Path,
    workflow_config: &WorkflowConfig,
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

    if let Some(ref mut state) = session.workflow_state {
        state.phase = Phase::Planning;
        state.iteration = 1;
        state.approval_overridden = false;
        state.objective = format!(
            "{}\n\nUSER FEEDBACK: The previous plan was reviewed and needs changes:\n{}",
            state.objective,
            user_feedback
        );
        let state_path = match planning_paths::state_path(working_dir, &state.feature_name) {
            Ok(p) => p,
            Err(e) => {
                session.handle_error(&format!("Failed to get state path: {}", e));
                return;
            }
        };
        state.set_updated_at();
        let _ = state.save(&state_path);

        let (new_approval_tx, new_approval_rx) =
            mpsc::channel::<UserApprovalResponse>(1);
        session.approval_tx = Some(new_approval_tx);

        let (new_control_tx, new_control_rx) =
            mpsc::channel::<WorkflowCommand>(1);
        session.workflow_control_tx = Some(new_control_tx);

        session.current_run_id += 1;
        let run_id = session.current_run_id;

        let cfg = workflow_config.clone();
        let new_handle = tokio::spawn({
            let state = state.clone();
            let working_dir = working_dir.to_path_buf();
            let tx = output_tx.clone();
            let sid = session.id;
            async move {
                run_workflow_with_config(
                    state,
                    WorkflowRunConfig {
                        working_dir,
                        state_path,
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
}

/// Handles a workflow that was stopped by the user.
/// Returns a ResumableSession if the snapshot was saved successfully.
fn handle_workflow_stopped(
    session: &mut crate::tui::Session,
    working_dir: &Path,
) -> Option<ResumableSession> {
    let mut snapshot_saved = false;
    if let Some(ref state) = session.workflow_state {
        match create_and_save_snapshot(session, state, working_dir) {
            Ok(path) => {
                session.add_output(format!("[planning] Session saved: {}", path.display()));
                snapshot_saved = true;
            }
            Err(e) => {
                session.add_output(format!("[planning] Warning: Failed to save: {}", e));
            }
        }
    }

    session.status = SessionStatus::Stopped;
    session.running = false;
    session.workflow_control_tx = None;
    session.add_output("".to_string());
    session.add_output("=== SESSION STOPPED ===".to_string());

    let session_info = session.workflow_state.as_ref().map(|state| {
        (
            state.workflow_session_id.clone(),
            state.feature_name.clone(),
        )
    });

    if let Some((workflow_session_id, feature_name)) = session_info {
        session.add_output(format!("Session ID: {}", workflow_session_id));
        let resume_cmd = build_resume_command(&workflow_session_id, working_dir);
        session.add_output(format!("To resume: {}", resume_cmd));

        if snapshot_saved {
            return Some(ResumableSession {
                feature_name,
                session_id: workflow_session_id,
                working_dir: working_dir.to_path_buf(),
            });
        }
    }

    None
}
