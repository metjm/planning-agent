//! Session browser input handling.
//!
//! This module handles keyboard input for the session browser overlay,
//! including navigation, resume, force-stop, and confirmation dialogs.

use crate::tui::session::context::{
    compute_effective_working_dir, validate_working_dir, SessionContext,
};
use crate::tui::{Event, InputMode, TabManager};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::Path;
use tokio::sync::mpsc;

/// Handle input when the session browser overlay is open.
pub async fn handle_session_browser_input(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    // Handle confirmation dialog input first
    if tab_manager.session_browser.confirmation_pending.is_some() {
        return handle_confirmation_input(key, tab_manager, working_dir, output_tx).await;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            tab_manager.session_browser.close();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            tab_manager.session_browser.select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            tab_manager.session_browser.select_prev();
        }
        KeyCode::Char('f') => {
            tab_manager.session_browser.toggle_filter();
        }
        KeyCode::Char('s') => {
            // Force-stop the selected session (with confirmation)
            if let Some(entry) = tab_manager.session_browser.selected_entry().cloned() {
                if entry.is_live {
                    tab_manager
                        .session_browser
                        .start_force_stop_confirmation(entry.session_id.clone());
                } else {
                    tab_manager.session_browser.error = Some("Session is not running".to_string());
                }
            }
        }
        KeyCode::Enter => {
            // Resume the selected session (with cross-directory check)
            if let Some(entry) = tab_manager.session_browser.selected_entry().cloned() {
                // Check if session is in a different directory
                if !entry.is_current_dir {
                    // Show cross-directory confirmation
                    tab_manager
                        .session_browser
                        .start_cross_directory_confirmation(
                            entry.session_id.clone(),
                            entry.working_dir.clone(),
                        );
                } else {
                    // Same directory - resume directly
                    resume_session_in_current_process(tab_manager, &entry, working_dir, output_tx);
                }
            }
        }
        KeyCode::Char('r') => {
            // Refresh the session list asynchronously
            trigger_refresh(tab_manager, working_dir, output_tx);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

/// Handle confirmation dialog input (y/n/Esc).
async fn handle_confirmation_input(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    use crate::tui::session_browser::ConfirmationState;

    let confirmation = tab_manager.session_browser.confirmation_pending.clone();

    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            // User confirmed
            match confirmation {
                Some(ConfirmationState::ForceStop { session_id }) => {
                    // Execute force-stop
                    execute_force_stop(tab_manager, &session_id, working_dir, output_tx).await;
                }
                Some(ConfirmationState::CrossDirectoryResume {
                    session_id,
                    target_dir,
                }) => {
                    // Resume in current process using session context for the target directory
                    execute_cross_directory_resume_in_process(
                        tab_manager,
                        &session_id,
                        &target_dir,
                        output_tx,
                    );
                }
                None => {}
            }
            tab_manager.session_browser.cancel_confirmation();
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // User cancelled
            tab_manager.session_browser.cancel_confirmation();
        }
        _ => {}
    }
    Ok(false)
}

/// Execute force-stop for a session.
async fn execute_force_stop(
    tab_manager: &mut TabManager,
    session_id: &str,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    // Use the new RPC client to force-stop
    let client = crate::session_daemon::RpcClient::new(false).await;
    if client.is_connected() {
        match client.force_stop(session_id).await {
            Ok(_) => {
                // Refresh the session list to show updated state
                trigger_refresh(tab_manager, working_dir, output_tx);
            }
            Err(e) => {
                tab_manager.session_browser.error = Some(format!("Force-stop failed: {}", e));
            }
        }
    } else {
        tab_manager.session_browser.error = Some("Daemon not connected".to_string());
    }
}

/// Execute cross-directory resume in the current process.
///
/// This function resumes a session from a different directory using session context
/// to track the session's working directory, state path, and configuration.
fn execute_cross_directory_resume_in_process(
    tab_manager: &mut TabManager,
    session_id: &str,
    target_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    // Find the entry for this session to get full information
    let entry = tab_manager
        .session_browser
        .entries
        .iter()
        .find(|e| e.session_id == session_id)
        .cloned();

    match entry {
        Some(entry) => {
            // Use the unified resume function which now handles cross-directory resume
            resume_session_in_current_process(tab_manager, &entry, target_dir, output_tx);
        }
        None => {
            // Entry not found - try to load directly
            if let Err(err) = validate_working_dir(target_dir) {
                tab_manager.session_browser.error = Some(err);
                return;
            }

            // Create a minimal entry for resume
            let entry = crate::tui::session_browser::SessionEntry {
                session_id: session_id.to_string(),
                feature_name: "Unknown".to_string(),
                phase: "Unknown".to_string(),
                iteration: 0,
                workflow_status: "Stopped".to_string(),
                liveness: crate::session_daemon::LivenessState::Stopped,
                last_seen_at: String::new(),
                last_seen_relative: String::new(),
                working_dir: target_dir.to_path_buf(),
                is_current_dir: false,
                has_snapshot: true, // Assume snapshot exists since we're trying to resume
                is_resumable: true,
                pid: None,
                is_live: false,
            };
            resume_session_in_current_process(tab_manager, &entry, target_dir, output_tx);
        }
    }
}

/// Resume a session in the current process.
///
/// This function now supports both same-directory and cross-directory resume
/// by creating a SessionContext with the appropriate working directories.
/// Workflow config is loaded from the snapshot's stored workflow name to ensure
/// the resumed session uses the same workflow that was originally used.
fn resume_session_in_current_process(
    tab_manager: &mut TabManager,
    entry: &crate::tui::session_browser::SessionEntry,
    _global_working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    tab_manager.session_browser.resuming = true;

    // Validate that the base working directory exists
    if let Err(err) = validate_working_dir(&entry.working_dir) {
        tab_manager.session_browser.error = Some(err);
        tab_manager.session_browser.resuming = false;
        return;
    }

    // Load the snapshot
    let load_result = crate::session_daemon::load_snapshot(&entry.session_id);

    match load_result {
        Ok(snapshot) => {
            // Load workflow config from snapshot's stored workflow name
            // This ensures the resumed session uses the same workflow that was originally used
            let workflow_config =
                crate::app::tui_runner::workflow_loading::load_workflow_from_snapshot(&snapshot);

            // Close the browser first to release the borrow
            tab_manager.session_browser.close();

            // Create a new tab for the resumed session
            let session = tab_manager.add_session_with_name(entry.feature_name.clone());
            let session_id = session.id;

            // Restore the session from snapshot
            let restored_view = snapshot.workflow_view.clone();
            *session = crate::tui::Session::from_ui_state(
                snapshot.ui_state.clone(),
                Some(restored_view.clone()),
            );
            session.id = session_id;
            session.adjust_start_time_for_previous_elapsed(snapshot.total_elapsed_before_resume_ms);

            // Compute effective_working_dir from worktree_info if present
            let effective_working_dir = compute_effective_working_dir(
                &snapshot.working_dir,
                restored_view.worktree_info.as_ref(),
            );

            // Create and set session context BEFORE starting the workflow
            let context = SessionContext::from_snapshot(
                snapshot.working_dir.clone(),
                snapshot.state_path.clone(),
                restored_view.worktree_info.as_ref(),
                workflow_config.clone(),
            );
            session.context = Some(context);

            // Log resume information
            session.add_output(format!("[planning] Resumed session: {}", entry.session_id));
            let feature_name = restored_view
                .feature_name
                .as_ref()
                .map(|f| f.0.as_str())
                .unwrap_or("<unknown>");
            let phase = restored_view
                .planning_phase
                .unwrap_or(crate::domain::types::Phase::Planning);
            let iteration = restored_view.iteration.map(|i| i.0).unwrap_or(1);
            session.add_output(format!(
                "[planning] Feature: {}, Phase: {:?}, Iteration: {}",
                feature_name, phase, iteration
            ));

            // Log working directory info if cross-directory or using worktree
            if !entry.is_current_dir {
                session.add_output(format!(
                    "[planning] Base directory: {}",
                    snapshot.working_dir.display()
                ));
            }
            if effective_working_dir != snapshot.working_dir {
                session.add_output(format!(
                    "[planning] Working in worktree: {}",
                    effective_working_dir.display()
                ));
            }

            // Set up for workflow continuation
            session.input_mode = InputMode::Normal;
            session.total_cost = snapshot.ui_state.total_cost;

            // Start the actual workflow
            let input = if let Some(ref workflow_id) = restored_view.workflow_id {
                crate::domain::WorkflowInput::Resume(crate::domain::ResumeWorkflowInput {
                    workflow_id: workflow_id.clone(),
                })
            } else {
                tab_manager.session_browser.error =
                    Some("Failed to resume: workflow ID missing from snapshot".to_string());
                tab_manager.session_browser.resuming = false;
                return;
            };

            super::super::workflow_lifecycle::start_resumed_workflow(
                session,
                input,
                restored_view,
                &snapshot.working_dir,
                &workflow_config,
                output_tx,
            );
        }
        Err(e) => {
            tab_manager.session_browser.error = Some(format!("Failed to load: {}", e));
            tab_manager.session_browser.resuming = false;
        }
    }
}

/// Trigger an async refresh of the session browser.
pub fn trigger_refresh(
    tab_manager: &mut TabManager,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    tab_manager.session_browser.loading = true;
    tab_manager.session_browser.error = None;

    let wd = working_dir.to_path_buf();
    let tx = output_tx.clone();

    tokio::spawn(async move {
        let (entries, daemon_connected, error) =
            crate::tui::session_browser::SessionBrowserState::refresh_async(&wd).await;
        let _ = tx.send(Event::SessionBrowserRefreshComplete {
            entries,
            daemon_connected,
            error,
        });
    });
}
