use crate::app::cli::Cli;
use crate::app::util::extract_feature_name;
use crate::app::workflow_common::pre_create_session_folder_with_working_dir;
use crate::domain::input::{NewWorkflowInput, WorkflowInput};
use crate::domain::types::WorkflowId;

use super::workflow_lifecycle::InitResult;
use crate::planning_paths;
use crate::tui::mention::update_mention_state;
use crate::tui::slash::update_slash_state;
use crate::tui::{Event, InputMode, SessionStatus, TabManager};
use crate::update;
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::Path;
use tokio::sync::mpsc;

use super::slash_commands::{apply_dangerous_defaults, parse_slash_command, SlashCommand};
use super::InitHandle;

pub(crate) async fn handle_naming_tab_input(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &Path,
    cli: &Cli,
    init_handle: &mut InitHandle,
    update_in_progress: bool,
) -> Result<bool> {
    if update_in_progress {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        return Ok(false);
    }

    // Clone file_index before getting mutable session reference
    let file_index = tab_manager.file_index.clone();
    let session = tab_manager.active_mut();

    // Handle @-mention dropdown navigation when active (takes priority over slash)
    if session.tab_mention_state.active && !session.tab_mention_state.matches.is_empty() {
        match key.code {
            KeyCode::Up => {
                session.tab_mention_state.select_prev();
                return Ok(false);
            }
            KeyCode::Down => {
                session.tab_mention_state.select_next();
                return Ok(false);
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.tab_mention_state.select_prev();
                return Ok(false);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.tab_mention_state.select_next();
                return Ok(false);
            }
            KeyCode::Tab | KeyCode::Enter
                if key.code == KeyCode::Tab || !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                session.accept_tab_mention();
                update_mention_state(
                    &mut session.tab_mention_state,
                    &session.tab_input,
                    session.tab_input_cursor,
                    &file_index,
                );
                return Ok(false);
            }
            KeyCode::Esc => {
                session.tab_mention_state.clear();
                return Ok(false);
            }
            _ => {}
        }
    }

    // Handle slash command dropdown navigation when active (only if mention not active)
    // Disabled when paste blocks exist
    if !session.has_tab_input_pastes()
        && session.tab_slash_state.active
        && !session.tab_slash_state.matches.is_empty()
    {
        match key.code {
            KeyCode::Up => {
                session.tab_slash_state.select_prev();
                return Ok(false);
            }
            KeyCode::Down => {
                session.tab_slash_state.select_next();
                return Ok(false);
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.tab_slash_state.select_prev();
                return Ok(false);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.tab_slash_state.select_next();
                return Ok(false);
            }
            KeyCode::Tab | KeyCode::Enter
                if key.code == KeyCode::Tab || !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                // If Enter is pressed and the input is already a valid slash command,
                // skip autocomplete acceptance and let it fall through to submit.
                // This allows Enter to execute `/update` without requiring a trailing space.
                let is_complete_slash_command = key.code == KeyCode::Enter
                    && parse_slash_command(session.tab_input.trim()).is_some();

                if !is_complete_slash_command {
                    session.accept_tab_slash();
                    update_slash_state(
                        &mut session.tab_slash_state,
                        &session.tab_input,
                        session.tab_input_cursor,
                    );
                    return Ok(false);
                }
                // Fall through to submit block for complete slash commands
            }
            KeyCode::Esc => {
                session.tab_slash_state.clear();
                return Ok(false);
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        KeyCode::Char('q') if session.tab_input.is_empty() => {
            return Ok(true);
        }
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            session.insert_tab_input_newline();
        }
        KeyCode::Enter if session.last_key_was_backslash => {
            session.delete_tab_input_char();
            session.insert_tab_input_newline();
            session.last_key_was_backslash = false;
        }
        KeyCode::Enter => {
            let has_content =
                !session.tab_input.trim().is_empty() || session.has_tab_input_pastes();
            let input_text = session.tab_input.trim().to_string();

            // Check for slash commands (only if no paste blocks)
            if !session.has_tab_input_pastes() {
                if let Some((cmd, _args)) = parse_slash_command(&input_text) {
                    // Clear input for all slash commands
                    session.tab_input.clear();
                    session.tab_input_cursor = 0;
                    session.tab_input_scroll = 0;
                    session.tab_mention_state.clear();
                    session.tab_slash_state.clear();

                    match cmd {
                        SlashCommand::Update => {
                            if let update::UpdateStatus::UpdateAvailable(_) =
                                &tab_manager.update_status
                            {
                                tab_manager.update_error = None;
                                tab_manager.update_in_progress = true;
                                tab_manager.update_spinner_frame = 0;

                                let update_tx = output_tx.clone();
                                tokio::spawn(async move {
                                    let result =
                                        tokio::task::spawn_blocking(update::perform_update)
                                            .await
                                            .unwrap_or_else(|_| {
                                                update::UpdateResult::InstallFailed(
                                                    "Update task panicked".to_string(),
                                                    false, // Not a feature-related error (task panic)
                                                )
                                            });
                                    // Receiver dropped means TUI is shutting down - safe to ignore
                                    let _ = update_tx.send(Event::UpdateInstallFinished(result));
                                });
                            } else {
                                tab_manager.update_error = Some("No update available".to_string());
                            }
                        }
                        SlashCommand::ConfigDangerous => {
                            // Clear any previous command state
                            tab_manager.command_error = None;
                            tab_manager.command_notice = None;
                            tab_manager.command_in_progress = true;

                            let cmd_tx = output_tx.clone();
                            tokio::spawn(async move {
                                let result = tokio::task::spawn_blocking(apply_dangerous_defaults)
                                    .await
                                    .map_err(|e| format!("Task panicked: {}", e));

                                // Receiver dropped means TUI is shutting down - safe to ignore
                                match result {
                                    Ok(config_result) => {
                                        let error = if config_result.has_errors() {
                                            Some("Some configurations failed".to_string())
                                        } else {
                                            None
                                        };
                                        let _ = cmd_tx.send(Event::SlashCommandResult {
                                            command: "config-dangerous".to_string(),
                                            summary: config_result.summary(),
                                            error,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = cmd_tx.send(Event::SlashCommandResult {
                                            command: "config-dangerous".to_string(),
                                            summary: String::new(),
                                            error: Some(e),
                                        });
                                    }
                                }
                            });
                        }
                        SlashCommand::Sessions => {
                            // Open the session browser overlay
                            tab_manager.session_browser.open(working_dir);
                        }
                        SlashCommand::MaxIterations(n) => {
                            if let Some(ref view) = session.workflow_view {
                                let old_value = view.max_iterations().map(|m| m.0).unwrap_or(0);
                                // Note: WorkflowView is read-only; max_iterations changes
                                // require a command dispatch (not yet implemented)
                                tab_manager.command_notice = Some(format!(
                                    "max-iterations: current={}, requested={} (read-only view)",
                                    old_value, n
                                ));
                            } else {
                                tab_manager.command_notice = Some(format!(
                                    "max-iterations set to {} (no active workflow)",
                                    n
                                ));
                            }
                        }
                        SlashCommand::Sequential(enabled) => {
                            if let Some(ref mut ctx) = session.context {
                                let mode = if enabled { "sequential" } else { "parallel" };
                                ctx.workflow_config.workflow.reviewing.sequential = enabled;
                                tab_manager.command_notice = Some(format!(
                                    "Review mode: {} (effective at next review phase)",
                                    mode
                                ));
                            } else {
                                tab_manager.command_notice =
                                    Some("No active workflow config".to_string());
                            }
                        }
                        SlashCommand::Aggregation(mode) => {
                            if let Some(ref mut ctx) = session.context {
                                let mode_str = match mode {
                                    crate::config::AggregationMode::AnyRejects => "any-rejects",
                                    crate::config::AggregationMode::AllReject => "all-reject",
                                    crate::config::AggregationMode::Majority => "majority",
                                };
                                ctx.workflow_config.workflow.reviewing.aggregation = mode;
                                tab_manager.command_notice = Some(format!(
                                    "Aggregation: {} (effective at next review phase)",
                                    mode_str
                                ));
                            } else {
                                tab_manager.command_notice =
                                    Some("No active workflow config".to_string());
                            }
                        }
                        SlashCommand::Workflow(name_opt) => {
                            // Get working directory from session context.
                            // We use base_working_dir (not effective_working_dir) because:
                            // 1. base_working_dir represents the user's original project directory
                            // 2. effective_working_dir may point to an ephemeral worktree
                            // 3. The user's requirement was "in one environment I can choose one
                            //    workflow, in another I can choose a different one"
                            let base_working_dir = session
                                .context
                                .as_ref()
                                .map(|ctx| ctx.base_working_dir.clone())
                                .unwrap_or_else(|| working_dir.to_path_buf());

                            match name_opt {
                                None => {
                                    // Open workflow browser overlay instead of printing text
                                    tab_manager.workflow_browser.open(&base_working_dir);
                                }
                                Some(name) => {
                                    // Select the specified workflow
                                    let workflows =
                                        crate::app::list_available_workflows_for_display()
                                            .unwrap_or_default();

                                    if let Some(found) = workflows.iter().find(|w| w.name == name) {
                                        let selection = crate::app::WorkflowSelection {
                                            workflow: name.clone(),
                                        };
                                        if let Err(e) = selection.save(&base_working_dir) {
                                            tab_manager.command_error =
                                                Some(format!("Failed to save selection: {}", e));
                                        } else {
                                            // Also update the active session's workflow config
                                            if let Some(ref mut ctx) = session.context {
                                                match crate::app::load_workflow_by_name(&name) {
                                                    Ok(config) => {
                                                        ctx.workflow_config = config;
                                                        tab_manager.command_notice = Some(format!(
                                                            "Workflow set to: {} ({})\nFor directory: {}",
                                                            name,
                                                            found.source,
                                                            base_working_dir.display()
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        tab_manager.command_error = Some(format!(
                                                            "Selected '{}' but failed to load config: {}",
                                                            name, e
                                                        ));
                                                    }
                                                }
                                            } else {
                                                tab_manager.command_notice = Some(format!(
                                                    "Workflow set to: {} ({})\nFor directory: {}",
                                                    name,
                                                    found.source,
                                                    base_working_dir.display()
                                                ));
                                            }
                                        }
                                    } else {
                                        tab_manager.command_error = Some(format!(
                                            "Unknown workflow: '{}'\nUse /workflow to list available workflows",
                                            name
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    return Ok(false);
                }
            }

            if has_content {
                let objective = session.get_submit_text_tab();
                session.tab_input.clear();
                session.tab_input_cursor = 0;
                session.tab_input_scroll = 0;
                session.clear_tab_input_pastes();
                session.input_mode = InputMode::Normal;
                session.status = SessionStatus::Planning;

                let session_id = session.id;
                let tx = output_tx.clone();
                let wd = working_dir.to_path_buf();
                let max_iter = cli.max_iterations;

                // Capture worktree-related CLI flags
                let worktree_flag = cli.worktree;
                let custom_worktree_dir = cli.worktree_dir.clone();
                let custom_worktree_branch = cli.worktree_branch.clone();

                let new_init_handle = tokio::spawn(async move {
                    // Receiver dropped means TUI is shutting down - safe to ignore for all sends in this block
                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: "[planning] Initializing...".to_string(),
                    });

                    let feature_name = extract_feature_name(&objective, Some(&tx)).await?;

                    let state_path = planning_paths::state_path(&wd, &feature_name)?;

                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: format!("[planning] Starting new workflow: {}", feature_name),
                    });
                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: format!("[planning] Objective: {}", objective),
                    });

                    // Generate workflow ID and create input for new workflow
                    let workflow_id = WorkflowId::new();
                    let workflow_session_id = workflow_id.to_string();
                    let mut input =
                        NewWorkflowInput::new(feature_name.clone(), objective.clone(), max_iter);

                    // Set up git worktree if enabled via --worktree
                    let effective_working_dir = if !worktree_flag {
                        // Worktree is disabled by default
                        wd.clone()
                    } else {
                        // Get session directory for worktree
                        let session_dir =
                            match crate::planning_paths::session_dir(&workflow_session_id) {
                                Ok(dir) => dir,
                                Err(e) => {
                                    // Receiver dropped means TUI is shutting down - safe to ignore
                                    let _ = tx.send(Event::SessionOutput {
                                        session_id,
                                        line: format!(
                                        "[planning] Warning: Could not get session directory: {}",
                                        e
                                    ),
                                    });
                                    // View will be created via CQRS when WorkflowCreated event is emitted
                                    return Ok::<_, anyhow::Error>(InitResult {
                                        input: WorkflowInput::New(input),
                                        view: None,
                                        state_path,
                                        feature_name,
                                        effective_working_dir: wd.clone(),
                                    });
                                }
                            };

                        let worktree_base = custom_worktree_dir
                            .as_ref()
                            .map(|d| d.to_path_buf())
                            .unwrap_or(session_dir);

                        match crate::git_worktree::create_session_worktree(
                            &wd,
                            &workflow_session_id,
                            &feature_name,
                            &worktree_base,
                            custom_worktree_branch.as_deref(),
                        ) {
                            crate::git_worktree::WorktreeSetupResult::Created(info) => {
                                // Receiver dropped means TUI is shutting down - safe to ignore
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!(
                                        "[planning] Created git worktree at: {}",
                                        info.worktree_path.display()
                                    ),
                                });
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!(
                                        "[planning] Working on branch: {}",
                                        info.branch_name
                                    ),
                                });
                                if let Some(ref source) = info.source_branch {
                                    let _ = tx.send(Event::SessionOutput {
                                        session_id,
                                        line: format!("[planning] Will merge into: {}", source),
                                    });
                                }
                                if info.has_submodules {
                                    let _ = tx.send(Event::SessionOutput {
                                        session_id,
                                        line: "[planning] Warning: Repository has submodules"
                                            .to_string(),
                                    });
                                }
                                let wt_state = crate::domain::types::WorktreeState::new(
                                    info.worktree_path.clone(),
                                    info.branch_name,
                                    info.source_branch,
                                    info.original_dir,
                                );
                                input.worktree_info = Some(wt_state);
                                info.worktree_path
                            }
                            crate::git_worktree::WorktreeSetupResult::NotAGitRepo => {
                                // Receiver dropped means TUI is shutting down - safe to ignore
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line:
                                        "[planning] Not a git repository, using original directory"
                                            .to_string(),
                                });
                                wd.clone()
                            }
                            crate::git_worktree::WorktreeSetupResult::Failed(err) => {
                                // Receiver dropped means TUI is shutting down - safe to ignore
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!(
                                        "[planning] Warning: Git worktree setup failed: {}",
                                        err
                                    ),
                                });
                                wd.clone()
                            }
                        }
                    };

                    // Pre-create plan folder and files (in ~/.planning-agent/sessions/)
                    pre_create_session_folder_with_working_dir(
                        &input,
                        &workflow_id,
                        Some(&effective_working_dir),
                    )
                    .context("Failed to pre-create plan files")?;

                    // View will be created via CQRS when WorkflowCreated event is emitted
                    Ok::<_, anyhow::Error>(InitResult {
                        input: WorkflowInput::New(input),
                        view: None,
                        state_path,
                        feature_name,
                        effective_working_dir,
                    })
                });

                *init_handle = Some((session_id, new_init_handle));
            }
        }
        KeyCode::Esc => {
            tab_manager.update_error = None;
            tab_manager.command_notice = None;
            tab_manager.command_error = None;
            tab_manager.close_current_if_empty();
        }
        KeyCode::Char(c) => {
            session.insert_tab_input_char(c);
            session.last_key_was_backslash = c == '\\';
            tab_manager.update_error = None;
            tab_manager.command_notice = None;
            tab_manager.command_error = None;
        }
        KeyCode::Backspace => {
            session.last_key_was_backslash = false;
            if !session.delete_paste_at_cursor_tab() {
                session.delete_tab_input_char();
            }
        }
        KeyCode::Left => {
            session.move_tab_input_cursor_left();
        }
        KeyCode::Right => {
            session.move_tab_input_cursor_right();
        }
        KeyCode::Up => {
            session.move_tab_input_cursor_up();
        }
        KeyCode::Down => {
            session.move_tab_input_cursor_down();
        }
        _ => {}
    }

    // Update @-mention state after any input change
    let session = tab_manager.active_mut();
    update_mention_state(
        &mut session.tab_mention_state,
        &session.tab_input,
        session.tab_input_cursor,
        &file_index,
    );

    // Update slash command state after any input change (only if no paste blocks)
    if !session.has_tab_input_pastes() {
        update_slash_state(
            &mut session.tab_slash_state,
            &session.tab_input,
            session.tab_input_cursor,
        );
    } else {
        session.tab_slash_state.clear();
    }

    Ok(false)
}
