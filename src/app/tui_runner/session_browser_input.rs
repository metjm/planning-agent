//! Session browser input handling.
//!
//! This module handles keyboard input for the session browser overlay,
//! including navigation, resume, force-stop, and confirmation dialogs.

use crate::config::WorkflowConfig;
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
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    // Handle confirmation dialog input first
    if tab_manager.session_browser.confirmation_pending.is_some() {
        return handle_confirmation_input(key, tab_manager, working_dir, workflow_config, output_tx)
            .await;
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
                    resume_session_in_current_process(
                        tab_manager,
                        &entry,
                        workflow_config,
                        output_tx,
                    );
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
    _workflow_config: &WorkflowConfig,
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
                    // Spawn new terminal for cross-directory resume
                    execute_cross_directory_resume(tab_manager, &session_id, &target_dir);
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
    // Use the session daemon client to force-stop
    let client = crate::session_daemon::client::SessionDaemonClient::new(false);
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

/// Execute cross-directory resume by spawning a new terminal.
fn execute_cross_directory_resume(
    tab_manager: &mut TabManager,
    session_id: &str,
    target_dir: &Path,
) {
    match crate::tui::terminal_spawn::spawn_terminal_for_resume(target_dir, session_id) {
        Ok(()) => {
            // Successfully spawned - close the browser
            tab_manager.session_browser.close();
        }
        Err(e) => {
            tab_manager.session_browser.error = Some(format!("Failed to spawn terminal: {}", e));
        }
    }
}

/// Resume a session in the current process (same directory).
fn resume_session_in_current_process(
    tab_manager: &mut TabManager,
    entry: &crate::tui::session_browser::SessionEntry,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    tab_manager.session_browser.resuming = true;

    // Load the snapshot
    let load_result = crate::session_store::load_snapshot(&entry.working_dir, &entry.session_id);

    match load_result {
        Ok(snapshot) => {
            // Close the browser first to release the borrow
            tab_manager.session_browser.close();

            // Create a new tab for the resumed session
            let session = tab_manager.add_session_with_name(entry.feature_name.clone());
            let session_id = session.id;

            // Restore the session from snapshot
            let restored_state = snapshot.workflow_state.clone();
            *session = crate::tui::Session::from_ui_state(
                snapshot.ui_state.clone(),
                Some(restored_state.clone()),
            );
            session.id = session_id; // Preserve the new session ID
            session.add_output(format!(
                "[planning] Resumed session: {}",
                entry.session_id
            ));
            session.add_output(format!(
                "[planning] Feature: {}, Phase: {:?}, Iteration: {}",
                restored_state.feature_name, restored_state.phase, restored_state.iteration
            ));

            // Set up for workflow continuation
            session.input_mode = InputMode::Normal;
            session.total_cost = snapshot.ui_state.total_cost;

            // Start the actual workflow
            super::workflow_lifecycle::start_resumed_workflow(
                session,
                restored_state,
                snapshot.state_path,
                &entry.working_dir,
                workflow_config,
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
