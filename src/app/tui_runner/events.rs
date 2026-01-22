use crate::app::util::shorten_model_name;
use crate::config::WorkflowConfig;
use crate::session_daemon;
use crate::tui::{
    ApprovalMode, Event, FocusedPanel, SessionStatus, TabManager, ToolKind, ToolResultSummary,
    ToolTimelineEntry,
};
use crate::update;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

use super::input::handle_key_event;
use super::snapshot_helper::create_and_save_snapshot;
use super::{restore_terminal, InitHandle};

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
            // Resize any active implementation terminals
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
        if session.status == SessionStatus::GeneratingSummary {
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
    summary: ToolResultSummary,
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

async fn handle_session_event(
    event: Event,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &Path,
) -> Result<()> {
    match event {
        Event::SessionOutput { session_id, line } => {
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
        Event::SessionStreaming { session_id, line } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.add_streaming(line);
            }
        }
        Event::SessionStateUpdate { session_id, state } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.name = state.feature_name.clone();
                session.workflow_state = Some(state);
            }
        }
        Event::SessionApprovalRequest {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_approval(summary);
            }
        }
        Event::SessionReviewDecisionRequest {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_review_decision(summary);
            }
        }
        Event::SessionReviewRoundStarted { session_id, round } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_review_round(round);
            }
        }
        Event::SessionReviewerStarted {
            session_id,
            round,
            display_id,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_started(round, display_id);
            }
        }
        Event::SessionReviewerCompleted {
            session_id,
            round,
            display_id,
            approved,
            summary,
            duration_ms,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_completed(round, display_id, approved, summary, duration_ms);
            }
        }
        Event::SessionReviewerFailed {
            session_id,
            round,
            display_id,
            error,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_failed(round, display_id, error);
            }
        }
        Event::SessionReviewRoundCompleted {
            session_id,
            round,
            approved,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.set_round_verdict(round, approved);
            }
        }
        Event::SessionTokenUsage { session_id, usage } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.add_token_usage(&usage);
            }
        }
        Event::SessionToolStarted {
            session_id,
            tool_id,
            display_name,
            input_preview,
            agent_name,
            phase,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
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
        Event::SessionToolFinished {
            session_id,
            tool_id,
            agent_name,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.tool_finished_for_agent(tool_id.as_deref(), &agent_name);
            }
        }
        Event::SessionBytesReceived { session_id, bytes } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.add_bytes(bytes);
            }
        }
        Event::SessionPhaseStarted { session_id, phase } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_phase(phase);

                // Save snapshot on phase transition (natural checkpoint for recovery)
                // Use session context's base_working_dir if available
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
        Event::SessionTurnCompleted { session_id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.turn_count += 1;
            }
        }
        Event::SessionModelDetected { session_id, name } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                if session.model_name.is_none() {
                    session.model_name = Some(shorten_model_name(&name));
                }
            }
        }
        Event::SessionToolResultReceived {
            session_id,
            tool_id,
            is_error,
            agent_name,
            phase,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                let info = session.tool_result_received_for_agent(
                    tool_id.as_deref(),
                    is_error,
                    &agent_name,
                );
                if info.is_error {
                    session.tool_error_count += 1;
                }
                session.total_tool_duration_ms += info.duration_ms;
                session.completed_tool_count += 1;
                session.add_tool_entry(
                    &phase,
                    ToolTimelineEntry::Finished {
                        agent_name: agent_name.clone(),
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
        Event::SessionStopReason { session_id, reason } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.last_stop_reason = Some(reason);
            }
        }
        Event::SessionWorkflowComplete { session_id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.status = SessionStatus::Complete;
                session.running = false;

                // Save a snapshot for completed sessions so they appear in /sessions history
                // Use session context's base_working_dir if available
                if let Some(ref state) = session.workflow_state {
                    let base_working_dir = session
                        .context
                        .as_ref()
                        .map(|ctx| ctx.base_working_dir.as_path())
                        .unwrap_or(working_dir);
                    if let Err(e) = create_and_save_snapshot(session, state, base_working_dir) {
                        session.add_output(format!(
                            "[planning] Warning: Failed to save completion snapshot: {}",
                            e
                        ));
                    }
                }
            }
        }
        Event::SessionWorkflowError { session_id, error } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_error(&error);
            }
        }
        Event::SessionGeneratingSummary { session_id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.status = SessionStatus::GeneratingSummary;
                session.spinner_frame = 0;
            }
        }
        Event::SessionPlanGenerationFailed { session_id, error } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_plan_generation_failed(error);
            }
        }
        Event::SessionMaxIterationsReached {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_max_iterations_prompt(summary);
            }
        }
        Event::SessionUserOverrideApproval {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_user_override_approval(summary);
            }
        }
        Event::SessionAllReviewersFailed {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_all_reviewers_failed(summary);
            }
        }
        Event::SessionWorkflowFailure {
            session_id,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_workflow_failure(summary);
            }
        }
        Event::SessionAgentMessage {
            session_id,
            agent_name,
            phase,
            message,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.add_chat_message(&agent_name, &phase, message);
            }
        }
        Event::SessionTodosUpdate {
            session_id,
            agent_name,
            todos,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.update_todos(agent_name, todos);
            }
        }
        Event::SessionRunTabSummaryGenerating {
            session_id,
            phase,
            run_id,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                // Only process if run_id matches current run
                if session.current_run_id == run_id {
                    session.set_summary_generating(&phase);
                }
            }
        }
        Event::SessionRunTabSummaryReady {
            session_id,
            phase,
            summary,
            run_id,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                // Only process if run_id matches current run
                if session.current_run_id == run_id {
                    session.set_summary_ready(&phase, summary);
                }
            }
        }
        Event::SessionRunTabSummaryError {
            session_id,
            phase,
            error,
            run_id,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                // Only process if run_id matches current run
                if session.current_run_id == run_id {
                    session.set_summary_error(&phase, error);
                }
            }
        }
        Event::AccountUsageUpdate(usage) => {
            for session in tab_manager.sessions_mut() {
                session.account_usage = usage.clone();
            }
        }
        Event::UpdateStatusReceived(status) => {
            tab_manager.update_status = status;
        }
        Event::VersionInfoReceived(info) => {
            tab_manager.version_info = info;
        }
        Event::FileIndexReady(index) => {
            tab_manager.file_index = index;
        }
        Event::SlashCommandResult {
            session_id: _,
            command,
            summary,
            error,
        } => {
            tab_manager.command_in_progress = false;
            if let Some(err) = error {
                tab_manager.command_error = Some(format!("/{}: {}", command, err));
                // Still show summary if available
                if !summary.is_empty() {
                    tab_manager.command_notice = Some(summary);
                }
            } else {
                tab_manager.command_notice = Some(summary);
                tab_manager.command_error = None;
            }
        }
        // Handle verification workflow events
        Event::SessionVerificationStarted {
            session_id,
            iteration,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_verification(iteration);
            }
        }
        Event::SessionVerificationCompleted {
            session_id,
            verdict,
            report,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_verification_completed(&verdict, &report);
            }
        }
        Event::SessionFixingStarted {
            session_id,
            iteration,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_fixing(iteration);
            }
        }
        Event::SessionFixingCompleted { session_id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_fixing_completed();
            }
        }
        Event::SessionVerificationResult {
            session_id,
            approved,
            iterations_used,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.handle_verification_result(approved, iterations_used);
            }
        }
        Event::SessionBrowserRefreshComplete {
            entries,
            daemon_connected,
            error,
        } => {
            tab_manager
                .session_browser
                .apply_refresh(entries, daemon_connected, error);
            tab_manager.daemon_connected = daemon_connected;
        }
        Event::DaemonSessionChanged(record) => {
            // Push notification: update session browser with new session state
            tab_manager.session_browser.apply_session_update(record);
        }
        Event::DaemonDisconnected => {
            tab_manager.session_browser.daemon_connected = false;
            tab_manager.daemon_connected = false;
        }
        Event::DaemonReconnected => {
            tab_manager.session_browser.daemon_connected = true;
            tab_manager.daemon_connected = true;
        }
        Event::UpdateInstallFinished(result) => {
            tab_manager.update_in_progress = false;

            match result {
                update::UpdateResult::Success(binary_path) => {
                    let _ = update::write_update_marker(working_dir);

                    // Shutdown the session daemon before exec'ing new binary
                    // This ensures the daemon persists its registry and exits cleanly
                    // The new binary will spawn a fresh daemon on startup
                    let client = session_daemon::client::SessionDaemonClient::new(false);
                    if client.is_connected() {
                        let _ = client.shutdown().await;
                        // Give daemon a moment to persist registry and exit
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }

                    restore_terminal(terminal)?;

                    let args: Vec<String> = std::env::args().skip(1).collect();

                    #[cfg(unix)]
                    {
                        use std::os::unix::process::CommandExt;
                        let err = std::process::Command::new(&binary_path).args(&args).exec();
                        eprintln!("Failed to exec new binary: {}", err);
                        std::process::exit(1);
                    }

                    #[cfg(not(unix))]
                    {
                        let _ = std::process::Command::new(&binary_path).args(&args).spawn();
                        std::process::exit(0);
                    }
                }
                update::UpdateResult::GitNotFound => {
                    tab_manager.update_error =
                        Some("Update requires git. Please install git and try again.".to_string());
                }
                update::UpdateResult::CargoNotFound => {
                    tab_manager.update_error = Some(
                        "Update requires cargo. Please install Rust and try again.".to_string(),
                    );
                }
                update::UpdateResult::InstallFailed(err) => {
                    let short_err = if err.len() > 60 {
                        format!("{}...", err.get(..57).unwrap_or(&err))
                    } else {
                        err
                    };
                    tab_manager.update_error = Some(short_err);
                }
                update::UpdateResult::BinaryNotFound => {
                    tab_manager.update_error =
                        Some("Update installed but binary not found".to_string());
                }
            }
        }
        // CLI instance lifecycle events
        Event::SessionCliInstanceStarted {
            session_id,
            id,
            agent_name,
            pid,
            started_at,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.cli_instance_started(id, agent_name, pid, started_at);
            }
        }
        Event::SessionCliInstanceActivity {
            session_id,
            id,
            activity_at,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.cli_instance_activity(id, activity_at);
            }
        }
        Event::SessionCliInstanceFinished { session_id, id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.cli_instance_finished(id);
            }
        }
        Event::SessionImplementationSuccess {
            session_id,
            iterations_used,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.open_implementation_success(iterations_used);
            }
        }
        Event::SessionImplementationInteractionFinished { session_id } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.implementation_interaction.running = false;
                session.implementation_interaction.cancel_tx = None;
            }
        }
        _ => {}
    }
    Ok(())
}
