use crate::app::util::shorten_model_name;
use crate::config::WorkflowConfig;
use crate::tui::{
    ApprovalMode, Event, FocusedPanel, SessionStatus, TabManager, ToolKind, ToolTimelineEntry,
};
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use super::input::handle_key_event;
use super::session_events::handle_session_event;
use super::snapshot_helper::create_and_save_snapshot;
use super::InitHandle;

#[allow(clippy::too_many_arguments)]
pub async fn process_event(
    event: Event,
    tab_manager: &mut TabManager,
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
        Event::Streaming(line) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.add_streaming(line);
            }
        }
        Event::ToolStarted {
            tool_id,
            display_name,
            input_preview,
            agent_name,
            phase,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.tool_started(
                    tool_id,
                    display_name.clone(),
                    input_preview.clone(),
                    agent_name.clone(),
                );
                session.tool_call_count += 1;
                session.add_tool_entry(
                    &phase,
                    ToolTimelineEntry::Started {
                        agent_name,
                        kind: ToolKind::from_display_name(&display_name),
                        display_name,
                        input_preview,
                    },
                );
            }
        }
        Event::ToolFinished {
            tool_id,
            agent_name,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.tool_finished_for_agent(tool_id.as_deref(), &agent_name);
            }
        }
        Event::StateUpdate(new_state) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.name = new_state.feature_name.clone();
                session.workflow_state = Some(new_state);
            }
        }
        Event::RequestUserApproval(summary) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.start_approval(summary);
            }
        }
        Event::BytesReceived(bytes) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.add_bytes(bytes);
            }
        }
        Event::TokenUsage(usage) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.add_token_usage(&usage);
            }
        }
        Event::PhaseStarted(phase) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.start_phase(phase);

                // Save snapshot on phase transition (natural checkpoint for recovery)
                // Use session context's base_working_dir if available, otherwise fall back to global
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
        Event::TurnCompleted => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.turn_count += 1;
            }
        }
        Event::ModelDetected(name) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                if session.model_name.is_none() {
                    session.model_name = Some(shorten_model_name(&name));
                }
            }
        }
        Event::ToolResultReceived {
            tool_id,
            is_error,
            agent_name,
            phase,
            summary,
        } => {
            handle_tool_result(
                first_session_id,
                tool_id.as_deref(),
                is_error,
                &agent_name,
                &phase,
                summary,
                tab_manager,
            );
        }
        Event::StopReason(reason) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.last_stop_reason = Some(reason);
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

fn handle_tool_result(
    session_id: usize,
    tool_id: Option<&str>,
    is_error: bool,
    agent_name: &str,
    phase: &str,
    summary: crate::tui::ToolResultSummary,
    tab_manager: &mut TabManager,
) {
    if let Some(session) = tab_manager.session_by_id_mut(session_id) {
        let info = session.tool_result_received_for_agent(tool_id, is_error, agent_name);
        if info.is_error {
            session.tool_error_count += 1;
        }
        session.total_tool_duration_ms += info.duration_ms;
        session.completed_tool_count += 1;
        session.add_tool_entry(
            phase,
            ToolTimelineEntry::Finished {
                agent_name: agent_name.to_string(),
                kind: ToolKind::from_display_name(&info.display_name),
                display_name: info.display_name,
                input_preview: info.input_preview,
                duration_ms: info.duration_ms,
                is_error: info.is_error,
                result_summary: summary,
            },
        );
    }
}
