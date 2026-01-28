use crate::tui::scroll_regions::ScrollableRegions;
use crate::tui::{ApprovalMode, Event, FocusedPanel, SessionStatus, TabManager};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use super::input::handle_key_event;
use super::input::mouse_input::{handle_mouse_click, handle_mouse_scroll};
use super::input::{is_summary_panel_visible, is_todo_panel_visible};
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
                init_handle,
            )
            .await?;
        }
        Event::Mouse(mouse) => {
            // Check browser overlay states first (on TabManager, not Session)
            let browser_overlay_active =
                tab_manager.session_browser.open || tab_manager.workflow_browser.open;

            let session = tab_manager.active_mut();

            // Skip click handling when any overlay/modal is active (they capture input)
            let modal_active = browser_overlay_active
                || session.error_state.is_some()
                || session.plan_modal_open
                || session.review_modal_open
                || session.implementation_success_modal.is_some()
                || session.approval_mode != ApprovalMode::None;

            if !modal_active {
                use crossterm::event::{MouseButton, MouseEventKind};
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                    let todos_visible = is_todo_panel_visible(session);
                    let summary_visible = is_summary_panel_visible(session);
                    handle_mouse_click(
                        mouse,
                        session,
                        scroll_regions,
                        todos_visible,
                        summary_visible,
                    );
                }
            }

            // Scroll handling works even with modals (scroll within modal)
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
        Event::SnapshotRequest => {
            // Save snapshot for all active sessions (periodic auto-save)
            // Use each session's context base_working_dir if available
            for session in tab_manager.sessions_mut() {
                if session.workflow_handle.is_some() {
                    if let Some(ref view) = session.workflow_view {
                        let base_working_dir = session
                            .context
                            .as_ref()
                            .map(|ctx| ctx.base_working_dir.as_path())
                            .unwrap_or(working_dir);
                        // Periodic snapshot failure is non-fatal - will retry on next interval
                        if let Err(e) = create_and_save_snapshot(session, view, base_working_dir) {
                            eprintln!(
                                "[planning] Warning: Failed to save periodic snapshot: {}",
                                e
                            );
                        }
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
                SessionStatus::Planning | SessionStatus::GeneratingSummary
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
            // Receiver dropped means TUI is shutting down - safe to ignore
            let _ = tx.send(Event::SessionBrowserRefreshComplete {
                entries,
                daemon_connected,
                error,
            });
        });
    }
}

fn handle_resize_event(tab_manager: &mut TabManager) {
    // On resize, reset scroll positions to prevent out-of-bounds rendering
    // The actual dimensions will be queried fresh on next draw
    let session = tab_manager.active_mut();

    // Clamp scroll positions - they'll be properly recalculated on next render
    // but this prevents crashes from stale large values
    session.plan_summary_scroll = 0;
    session.plan_modal_scroll = 0;
    session.review_modal_scroll = 0;
    session.error_scroll = 0;

    // Reset run tab scrolls
    for tab in &mut session.run_tabs {
        tab.chat_scroll = crate::tui::scroll_state::ScrollState::new();
        tab.summary_scroll = crate::tui::scroll_state::ScrollState::new();
    }
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
