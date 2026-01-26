//! Session-specific event handlers extracted from events.rs.
//!
//! This module handles all `Event::Session*` variants to keep the main
//! events module under the 750-line limit.

use crate::app::util::shorten_model_name;
use crate::session_daemon;
use crate::tui::{Event, TabManager, ToolKind, ToolTimelineEntry};
use crate::update;
use anyhow::Result;
use std::path::Path;

use super::restore_terminal;
use super::snapshot_helper::create_and_save_snapshot;

/// Handle session-specific events (Event::Session* variants).
pub async fn handle_session_event(
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
        Event::SessionViewUpdate { session_id, view } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                let view = *view; // Unbox the view
                if let Some(ref name) = view.feature_name {
                    session.name = name.as_str().to_string();
                }
                session.workflow_view = Some(view);
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
        Event::SessionReviewRoundStarted {
            session_id,
            kind,
            round,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_review_round(kind, round);
            }
        }
        Event::SessionReviewerStarted {
            session_id,
            kind,
            round,
            display_id,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_started(kind, round, display_id);
            }
        }
        Event::SessionReviewerCompleted {
            session_id,
            kind,
            round,
            display_id,
            approved,
            summary,
            duration_ms,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_completed(kind, round, display_id, approved, summary, duration_ms);
            }
        }
        Event::SessionReviewerFailed {
            session_id,
            kind,
            round,
            display_id,
            error,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.reviewer_failed(kind, round, display_id, error);
            }
        }
        Event::SessionReviewRoundCompleted {
            session_id,
            kind,
            round,
            approved,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.set_round_verdict(kind, round, approved);
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
                if let Some(ref view) = session.workflow_view {
                    let base_working_dir = session
                        .context
                        .as_ref()
                        .map(|ctx| ctx.base_working_dir.as_path())
                        .unwrap_or(working_dir);
                    let _ = create_and_save_snapshot(session, view, base_working_dir);
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
        Event::SessionPlanGenerationFailed { session_id, error } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_plan_generation_failed(error);
            }
        }
        Event::SessionMaxIterationsReached {
            session_id,
            phase,
            summary,
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                // Include phase info in the output for context
                session.add_output(format!(
                    "[{}] Max iterations reached - awaiting decision",
                    phase.display_name()
                ));
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
            command,
            summary,
            error,
        } => {
            tab_manager.command_in_progress = false;
            if let Some(err) = error {
                tab_manager.command_error = Some(format!("/{}: {}", command, err));
                if !summary.is_empty() {
                    tab_manager.command_notice = Some(summary);
                }
            } else {
                tab_manager.command_notice = Some(summary);
                tab_manager.command_error = None;
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
            handle_update_install_finished(result, tab_manager, terminal).await?;
        }
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

async fn handle_update_install_finished(
    result: update::UpdateResult,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    tab_manager.update_in_progress = false;

    match result {
        update::UpdateResult::Success(binary_path, features_msg) => {
            let _ = update::write_update_marker();

            // Shutdown the session daemon before exec'ing new binary
            let client = session_daemon::RpcClient::new(false).await;
            if client.is_connected() {
                let _ = client.shutdown().await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            restore_terminal(terminal)?;

            // Log the features message for debugging (will be visible before exec)
            if !features_msg.is_empty() {
                eprintln!("Update installed successfully{}!", features_msg);
            }

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
            tab_manager.update_error =
                Some("Update requires cargo. Please install Rust and try again.".to_string());
        }
        update::UpdateResult::InstallFailed(err, is_feature_error) => {
            if is_feature_error && !update::BUILD_FEATURES.is_empty() {
                tab_manager.update_error = Some(format!(
                    "Update failed: features '{}' may no longer exist.\n\
                     Try: cargo install --git https://github.com/metjm/planning-agent.git --force",
                    update::BUILD_FEATURES,
                ));
            } else {
                let short_err = if err.len() > 60 {
                    format!("{}...", err.get(..57).unwrap_or(&err))
                } else {
                    err
                };
                tab_manager.update_error = Some(short_err);
            }
        }
        update::UpdateResult::BinaryNotFound => {
            tab_manager.update_error = Some("Update installed but binary not found".to_string());
        }
    }
    Ok(())
}
