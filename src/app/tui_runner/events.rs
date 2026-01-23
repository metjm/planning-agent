use crate::config::WorkflowConfig;
use crate::tui::scroll_regions::ScrollableRegions;
use crate::tui::{ApprovalMode, Event, FocusedPanel, SessionStatus, TabManager};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use super::input::handle_key_event;
use super::mouse_input::handle_mouse_scroll;
use super::session_events::handle_session_event;
use super::snapshot_helper::create_and_save_snapshot;
use super::InitHandle;

#[allow(clippy::too_many_arguments)]
pub async fn process_event(
    event: Event,
    tab_manager: &mut TabManager,
    scroll_regions: &ScrollableRegions,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &Path,
    cli: &crate::app::cli::Cli,
    workflow_config: &WorkflowConfig,
    init_handle: &mut InitHandle,
    first_session_id: usize,
) -> Result<bool> {
    let mut should_quit = false;

    match event {
        Event::Key(key) => {
            should_quit = handle_key_event(
                key,
                tab_manager,
                terminal,
                output_tx,
                working_dir,
                cli,
                workflow_config,
                init_handle,
            )
            .await?;
        }
        Event::Mouse(mouse) => {
            let session = tab_manager.active_mut();
            handle_mouse_scroll(mouse, session, scroll_regions);
        }
        Event::Tick => {
            handle_tick_event(tab_manager, output_tx, working_dir);
        }
        Event::Resize => {
            handle_resize_event(tab_manager);
        }
        Event::Paste(text) => {
            handle_paste_event(text, tab_manager);
        }

        Event::Output(line) => {
            handle_legacy_output(first_session_id, line, tab_manager);
        }
        Event::StateUpdate(new_state) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.name = new_state.feature_name.clone();
                session.workflow_state = Some(new_state);
            }
        }
        Event::SnapshotRequest => {
            // Save snapshot for all active sessions (periodic auto-save)
            // Use each session's context base_working_dir if available
            for session in tab_manager.sessions_mut() {
                if session.workflow_handle.is_some() {
                    if let Some(ref state) = session.workflow_state {
                        let base_working_dir = session
                            .context
                            .as_ref()
                            .map(|ctx| ctx.base_working_dir.as_path())
                            .unwrap_or(working_dir);
                        let _ = create_and_save_snapshot(session, state, base_working_dir);
                    }
                }
            }
        }
        _ => {
            handle_session_event(event, tab_manager, terminal, working_dir).await?;
        }
    }

    Ok(should_quit)
}

fn handle_tick_event(
    tab_manager: &mut TabManager,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &Path,
) {
    for session in tab_manager.sessions_mut() {
        // Advance spinner for any active/running state (header animation)
        if session.running
            || matches!(
                session.status,
                SessionStatus::Planning
                    | SessionStatus::GeneratingSummary
                    | SessionStatus::Verifying
                    | SessionStatus::Fixing
            )
        {
            session.spinner_frame = session.spinner_frame.wrapping_add(1);
        }
        session.advance_summary_spinners();
        session.advance_review_history_spinner();
    }
    if tab_manager.update_in_progress {
        tab_manager.update_spinner_frame = tab_manager.update_spinner_frame.wrapping_add(1);
    }

    // Advance spinner frame when session browser is open (for running session animations)
    if tab_manager.session_browser.open {
        tab_manager.update_spinner_frame = tab_manager.update_spinner_frame.wrapping_add(1);
    }

    // Auto-refresh session browser if open and due for refresh
    if tab_manager.session_browser.open && tab_manager.session_browser.should_auto_refresh() {
        tab_manager.session_browser.loading = true;
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
}

fn handle_resize_event(_tab_manager: &mut TabManager) {
    // No-op: terminal resize is handled automatically by ratatui
}

fn handle_paste_event(text: String, tab_manager: &mut TabManager) {
    use crate::tui::InputMode;
    let session = tab_manager.active_mut();
    if session.input_mode == InputMode::NamingTab {
        session.insert_paste_tab_input(text);
    } else if session.approval_mode == ApprovalMode::EnteringFeedback {
        session.insert_paste_feedback(text);
    } else if session.focused_panel == FocusedPanel::ChatInput {
        session.insert_paste_tab_input(text);
    }
}

fn handle_legacy_output(session_id: usize, line: String, tab_manager: &mut TabManager) {
    if let Some(session) = tab_manager.session_by_id_mut(session_id) {
        if line.contains("Cost: $") {
            if let Some(cost_str) = line.split('$').nth(1) {
                if let Ok(cost) = cost_str.trim().parse::<f64>() {
                    session.total_cost += cost;
                }
            }
        }
        session.add_output(line);
    }
}
