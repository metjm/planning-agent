
use crate::app::util::shorten_model_name;
use crate::app::workflow::WorkflowResult;
use crate::config::WorkflowConfig;
use crate::state::{Phase, State};
use crate::tui::{
    ApprovalMode, Event, InputMode, SessionStatus, TabManager, UserApprovalResponse,
    WorkflowCommand,
};
use crate::update;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::input::handle_key_event;
use super::{restore_terminal, run_workflow_with_config, InitHandle};

#[allow(clippy::too_many_arguments)]
pub async fn process_event(
    event: Event,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &PathBuf,
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
            handle_tick_event(tab_manager);
        }
        Event::Resize => {

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
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.tool_started(tool_id, display_name, input_preview, agent_name);
                session.tool_call_count += 1;
            }
        }
        Event::ToolFinished { tool_id, agent_name } => {
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
        } => {
            handle_tool_result(first_session_id, tool_id.as_deref(), is_error, &agent_name, tab_manager);
        }
        Event::StopReason(reason) => {
            if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                session.last_stop_reason = Some(reason);
            }
        }

        _ => {
            handle_session_event(event, tab_manager, terminal, working_dir).await?;
        }
    }

    Ok(should_quit)
}

fn handle_tick_event(tab_manager: &mut TabManager) {
    for session in tab_manager.sessions_mut() {
        if session.status == SessionStatus::GeneratingSummary {
            session.spinner_frame = session.spinner_frame.wrapping_add(1);
        }
        session.advance_summary_spinners();
    }
    if tab_manager.update_in_progress {
        tab_manager.update_spinner_frame = tab_manager.update_spinner_frame.wrapping_add(1);
    }
}

fn handle_paste_event(text: String, tab_manager: &mut TabManager) {
    let session = tab_manager.active_mut();
    if session.input_mode == InputMode::NamingTab {
        session.insert_paste_tab_input(text);
    } else if session.approval_mode == ApprovalMode::EnteringFeedback {
        session.insert_paste_feedback(text);
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
    tab_manager: &mut TabManager,
) {
    if let Some(session) = tab_manager.session_by_id_mut(session_id) {
        if is_error {
            session.tool_error_count += 1;
        }
        if let Some(duration_ms) = session.tool_result_received_for_agent(tool_id, is_error, agent_name) {
            session.total_tool_duration_ms += duration_ms;
            session.completed_tool_count += 1;
        }
    }
}

async fn handle_session_event(
    event: Event,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
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
        Event::SessionApprovalRequest { session_id, summary } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_approval(summary);
            }
        }
        Event::SessionReviewDecisionRequest { session_id, summary } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_review_decision(summary);
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
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.tool_started(tool_id, display_name, input_preview, agent_name);
                session.tool_call_count += 1;
            }
        }
        Event::SessionToolFinished { session_id, tool_id, agent_name } => {
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
        } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                if is_error {
                    session.tool_error_count += 1;
                }
                if let Some(duration_ms) = session.tool_result_received_for_agent(tool_id.as_deref(), is_error, &agent_name) {
                    session.total_tool_duration_ms += duration_ms;
                    session.completed_tool_count += 1;
                }
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
        Event::SessionMaxIterationsReached { session_id, summary } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_max_iterations_prompt(summary);
            }
        }
        Event::SessionUserOverrideApproval { session_id, summary } => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.start_user_override_approval(summary);
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
        Event::SessionRunTabSummaryGenerating { session_id, phase, run_id } => {
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
        Event::UpdateInstallFinished(result) => {
            tab_manager.update_in_progress = false;

            match result {
                update::UpdateResult::Success(binary_path) => {
                    let _ = update::write_update_marker(working_dir);

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
                        format!("{}...", &err[..57])
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
        _ => {}
    }
    Ok(())
}

pub async fn handle_init_completion(
    session_id: usize,
    handle: tokio::task::JoinHandle<anyhow::Result<(State, PathBuf, String)>>,
    tab_manager: &mut TabManager,
    working_dir: &PathBuf,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    match handle.await {
        Ok(Ok((state, state_path, feature_name))) => {
            if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                session.name = feature_name;
                session.workflow_state = Some(state.clone());

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
                    let working_dir = working_dir.clone();
                    let tx = output_tx.clone();
                    let sid = session_id;
                    async move {
                        run_workflow_with_config(
                            state,
                            working_dir,
                            state_path,
                            cfg,
                            tx,
                            new_approval_rx,
                            new_control_rx,
                            sid,
                            run_id,
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

pub async fn check_workflow_completions(
    tab_manager: &mut TabManager,
    working_dir: &PathBuf,
    workflow_config: &WorkflowConfig,
    output_tx: &mpsc::UnboundedSender<Event>,
) {
    for session in tab_manager.sessions_mut() {
        if let Some(handle) = session.workflow_handle.take() {
            let handle: tokio::task::JoinHandle<anyhow::Result<WorkflowResult>> = handle;
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(WorkflowResult::Accepted)) => {
                        session.status = SessionStatus::Complete;
                        session.running = false;
                    }
                    Ok(Ok(WorkflowResult::Aborted { reason })) => {
                        session.status = SessionStatus::Error;
                        session.running = false;
                        session.error_state = Some(reason);
                    }
                    Ok(Ok(WorkflowResult::NeedsRestart { user_feedback })) => {
                        session.add_output("".to_string());
                        session.add_output("=== RESTARTING WITH YOUR FEEDBACK ===".to_string());
                        session.add_output(format!("Changes requested: {}", user_feedback));

                        session.streaming_lines.clear();
                        session.clear_todos();
                        // Clear run tabs for clean restart
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
                            let state_path = working_dir
                                .join(format!(".planning-agent/{}.json", state.feature_name));
                            state.set_updated_at();
                            let _ = state.save(&state_path);

                            let (new_approval_tx, new_approval_rx) =
                                mpsc::channel::<UserApprovalResponse>(1);
                            session.approval_tx = Some(new_approval_tx);

                            // Create control channel for workflow interrupts
                            let (new_control_tx, new_control_rx) =
                                mpsc::channel::<WorkflowCommand>(1);
                            session.workflow_control_tx = Some(new_control_tx);

                            // Increment run_id to invalidate any stale summary events
                            session.current_run_id += 1;
                            let run_id = session.current_run_id;

                            let cfg = workflow_config.clone();
                            let new_handle = tokio::spawn({
                                let state = state.clone();
                                let working_dir = working_dir.clone();
                                let tx = output_tx.clone();
                                let sid = session.id;
                                async move {
                                    run_workflow_with_config(
                                        state,
                                        working_dir,
                                        state_path,
                                        cfg,
                                        tx,
                                        new_approval_rx,
                                        new_control_rx,
                                        sid,
                                        run_id,
                                    )
                                    .await
                                }
                            });

                            session.workflow_handle = Some(new_handle);
                        }
                    }
                    Ok(Ok(WorkflowResult::Stopped)) => {
                        // Save snapshot before marking as stopped
                        if let Some(ref state) = session.workflow_state {
                            let ui_state = session.to_ui_state();
                            let state_path = working_dir
                                .join(format!(".planning-agent/{}.json", state.feature_name));
                            let now = chrono::Utc::now().to_rfc3339();
                            let mut state_copy = state.clone();
                            state_copy.set_updated_at_with(&now);
                            let elapsed = session.start_time.elapsed().as_millis() as u64;

                            let snapshot = crate::session_store::SessionSnapshot::new_with_timestamp(
                                working_dir.clone(),
                                state.workflow_session_id.clone(),
                                state_path,
                                state_copy,
                                ui_state,
                                elapsed,
                                now,
                            );

                            match crate::session_store::save_snapshot(working_dir, &snapshot) {
                                Ok(path) => {
                                    session.add_output(format!("[planning] Session saved: {}", path.display()));
                                }
                                Err(e) => {
                                    session.add_output(format!("[planning] Warning: Failed to save: {}", e));
                                }
                            }
                        }

                        session.status = SessionStatus::Stopped;
                        session.running = false;
                        session.add_output("".to_string());
                        session.add_output("=== SESSION STOPPED ===".to_string());
                        if let Some(ref state) = session.workflow_state {
                            session.add_output(format!("Session ID: {}", state.workflow_session_id));
                            session.add_output("To resume: planning --resume-session <session-id>".to_string());
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
}
