use crate::app::cli::Cli;
use crate::app::headless::extract_feature_name;
use crate::app::workflow_common::pre_create_plan_files_with_working_dir;
use crate::config::WorkflowConfig;
use crate::planning_paths;
use crate::state::State;
use crate::tui::file_index::FileIndex;
use crate::tui::mention::update_mention_state;
use crate::tui::slash::update_slash_state;
use crate::tui::ui::util::{
    compute_summary_panel_inner_size, compute_wrapped_line_count,
    compute_wrapped_line_count_text,
};
use crate::tui::{
    ApprovalMode, Event, FeedbackTarget, FocusedPanel, InputMode, Session, SessionStatus,
    TabManager, WorkflowCommand,
};
use crate::update;
use anyhow::{Context, Result};

use super::slash_commands::{apply_dangerous_defaults, parse_slash_command, SlashCommand};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::text::Line;
use std::path::Path;
use tokio::sync::mpsc;

use super::approval_input::{handle_awaiting_choice_input, handle_entering_feedback_input};
use super::implementation_input::handle_implementation_terminal_input;
use super::InitHandle;
use crate::tui::ui::util::{compute_plan_modal_inner_size, parse_markdown_line};

/// Compute the max scroll for the run-tab summary panel based on wrapped lines and terminal size.
fn compute_run_tab_summary_max_scroll(summary_text: &str) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let (inner_width, visible_height) = compute_summary_panel_inner_size(term_width, term_height);

    let summary_lines: Vec<Line> = summary_text.lines().map(parse_markdown_line).collect();
    let total_lines = compute_wrapped_line_count(&summary_lines, inner_width);

    total_lines.saturating_sub(visible_height as usize)
}

/// Compute the inner size and visibility of the Todo panel given terminal dimensions.
///
/// This replicates the layout logic used in `draw_output`:
/// - Main layout: top bar (2), main content (min 0), footer (3)
/// - Main content split: 70% left, 30% right - we're in the left 70%
/// - Output area split: 40% for output
/// - When todos exist and width >= 80, output splits 65%/35% for output/todos
///
/// Returns (inner_width, inner_height, is_visible) of the Todo panel inner area.
fn compute_todo_panel_inner_size(
    terminal_width: u16,
    terminal_height: u16,
    has_todos: bool,
) -> (u16, u16, bool) {
    // Main layout: top bar (2) + footer (3) = 5 rows overhead
    let main_content_height = terminal_height.saturating_sub(5);

    // Horizontal split: 70% left, 30% right
    let left_width = (terminal_width as f32 * 0.70) as u16;

    // Vertical split: 40% output, 60% chat
    let output_height = (main_content_height as f32 * 0.40) as u16;

    // Todos are visible only when: output area width >= 80 AND todos exist
    let todos_visible = left_width >= 80 && has_todos;

    if !todos_visible {
        return (0, 0, false);
    }

    // Output area split: 65% output, 35% todos
    let todo_width = (left_width as f32 * 0.35) as u16;

    // Todo block has borders (1 row each for top/bottom, 1 col each for left/right)
    let inner_height = output_height.saturating_sub(2);
    let inner_width = todo_width.saturating_sub(2);

    (inner_width, inner_height, true)
}

/// Compute the max scroll for the Todo panel based on wrapped lines and terminal size.
fn compute_todo_panel_max_scroll(session: &Session) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let has_todos = !session.todos.is_empty();
    let (inner_width, inner_height, visible) =
        compute_todo_panel_inner_size(term_width, term_height, has_todos);

    if !visible || inner_width == 0 || inner_height == 0 {
        return 0;
    }

    let todo_lines = session.get_todos_display();
    let lines: Vec<Line> = todo_lines.iter().map(|s| Line::from(s.as_str())).collect();
    let total_lines = compute_wrapped_line_count(&lines, inner_width);

    total_lines.saturating_sub(inner_height as usize)
}

/// Check if the Todo panel is currently visible based on terminal size and todos.
fn is_todo_panel_visible(session: &Session) -> bool {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let has_todos = !session.todos.is_empty();
    let (_, _, visible) = compute_todo_panel_inner_size(term_width, term_height, has_todos);
    visible
}

/// Compute the max scroll for the plan modal based on wrapped lines and terminal size.
fn compute_plan_modal_max_scroll(content: &str) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let (inner_width, visible_height) = compute_plan_modal_inner_size(term_width, term_height);

    let content_lines: Vec<Line> = content.lines().map(parse_markdown_line).collect();
    let total_lines = compute_wrapped_line_count(&content_lines, inner_width);

    total_lines.saturating_sub(visible_height as usize)
}

/// Compute the visible height of the plan modal for page scrolling.
fn compute_plan_modal_visible_height() -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let (_, visible_height) = compute_plan_modal_inner_size(term_width, term_height);
    visible_height as usize
}

/// Compute the max scroll for the error overlay based on wrapped lines and terminal size.
fn compute_error_overlay_max_scroll(error: &str) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));

    // Match draw_error_overlay: 60% width, max 70
    let popup_width = (term_width as f32 * 0.6).min(70.0) as u16;
    let inner_width = popup_width.saturating_sub(2);

    // Compute wrapped line count for the error text
    let wrapped_error_lines = compute_wrapped_line_count_text(error, inner_width);

    // Popup height calculation (matching draw_error_overlay)
    let max_popup_height = (term_height as f32 * 0.8) as u16;
    let min_popup_height = 8u16;
    let ideal_popup_height = (wrapped_error_lines as u16).saturating_add(5);
    let popup_height = ideal_popup_height.clamp(min_popup_height, max_popup_height);

    // Visible height = popup_height - borders (2) - instructions (1)
    let visible_height = popup_height.saturating_sub(3) as usize;

    // Total content = empty line + error text + empty line
    let total_content_lines = wrapped_error_lines + 2;

    total_content_lines.saturating_sub(visible_height)
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_key_event(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &Path,
    cli: &Cli,
    workflow_config: &WorkflowConfig,
    init_handle: &mut InitHandle,
) -> Result<bool> {
    #[allow(unused_assignments)]
    let mut should_quit = false;

    let update_in_progress = tab_manager.update_in_progress;

    if tab_manager.active().input_mode == InputMode::NamingTab {
        tab_manager.update_notice = None;
    }

    // Handle session browser overlay input when it's open
    if tab_manager.session_browser.open {
        should_quit = super::session_browser_input::handle_session_browser_input(
            key, tab_manager, working_dir, workflow_config, output_tx
        ).await?;
        return Ok(should_quit);
    }

    let session = tab_manager.active_mut();

    if let Some(ref error) = session.error_state.clone() {
        match key.code {
            KeyCode::Esc => {
                session.clear_error();
                return Ok(false);
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                tab_manager.close_tab(tab_manager.active_tab);
                return Ok(false);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let max_scroll = compute_error_overlay_max_scroll(error);
                session.error_scroll_down(max_scroll);
                return Ok(false);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                session.error_scroll_up();
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    // Handle 'p' to toggle plan modal (global hotkey, works from any mode except error state or input areas)
    let in_text_input = session.input_mode != InputMode::Normal
        || session.approval_mode == ApprovalMode::EnteringFeedback;
    if key.code == KeyCode::Char('p') && session.workflow_state.is_some() && !in_text_input {
        session.toggle_plan_modal(working_dir);
        return Ok(false);
    }

    // Handle plan modal input when it's open (intercept keys before other handlers)
    if session.plan_modal_open {
        let content = session.plan_modal_content.clone();
        match key.code {
            KeyCode::Esc | KeyCode::Char('p') => {
                session.close_plan_modal();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let max_scroll = compute_plan_modal_max_scroll(&content);
                session.plan_modal_scroll_down(max_scroll);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                session.plan_modal_scroll_up();
            }
            KeyCode::Char('g') => {
                session.plan_modal_scroll_to_top();
            }
            KeyCode::Char('G') => {
                let max_scroll = compute_plan_modal_max_scroll(&content);
                session.plan_modal_scroll_to_bottom(max_scroll);
            }
            KeyCode::PageDown => {
                let visible_height = compute_plan_modal_visible_height();
                let max_scroll = compute_plan_modal_max_scroll(&content);
                session.plan_modal_page_down(visible_height, max_scroll);
            }
            KeyCode::PageUp => {
                let visible_height = compute_plan_modal_visible_height();
                session.plan_modal_page_up(visible_height);
            }
            _ => {}
        }
        return Ok(false);
    }

    if session.input_mode == InputMode::NamingTab {
        should_quit = handle_naming_tab_input(
            key,
            tab_manager,
            output_tx,
            working_dir,
            cli,
            init_handle,
            update_in_progress,
        )
        .await?;
        return Ok(should_quit);
    }

    // Handle implementation terminal input mode
    let session = tab_manager.active_mut();
    if session.input_mode == InputMode::ImplementationTerminal {
        should_quit = handle_implementation_terminal_input(key, session)?;
        return Ok(should_quit);
    }

    let session = tab_manager.active_mut();
    if session.approval_mode == ApprovalMode::None && handle_tab_switching(key, tab_manager) {
        return Ok(false);
    }

    // Clone file_index for mention handling
    let file_index = tab_manager.file_index.clone();
    let session = tab_manager.active_mut();
    should_quit = handle_approval_mode_input(key, session, terminal, working_dir, output_tx, workflow_config, &file_index).await?;

    Ok(should_quit)
}

async fn handle_naming_tab_input(
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
                if key.code == KeyCode::Tab
                    || !key.modifiers.contains(KeyModifiers::SHIFT) =>
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
                if key.code == KeyCode::Tab
                    || !key.modifiers.contains(KeyModifiers::SHIFT) =>
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
                            if let update::UpdateStatus::UpdateAvailable(_) = &tab_manager.update_status {
                                tab_manager.update_error = None;
                                tab_manager.update_in_progress = true;
                                tab_manager.update_spinner_frame = 0;

                                let update_tx = output_tx.clone();
                                tokio::spawn(async move {
                                    let result = tokio::task::spawn_blocking(update::perform_update)
                                        .await
                                        .unwrap_or_else(|_| {
                                            update::UpdateResult::InstallFailed(
                                                "Update task panicked".to_string(),
                                            )
                                        });
                                    let _ = update_tx.send(Event::UpdateInstallFinished(result));
                                });
                            } else {
                                tab_manager.update_error = Some("No update available".to_string());
                            }
                        }
                        SlashCommand::ConfigDangerous => {
                            // Get session_id before accessing tab_manager
                            let session_id = session.id;

                            // Clear any previous command state
                            tab_manager.command_error = None;
                            tab_manager.command_notice = None;
                            tab_manager.command_in_progress = true;

                            let cmd_tx = output_tx.clone();
                            tokio::spawn(async move {
                                let result = tokio::task::spawn_blocking(apply_dangerous_defaults)
                                    .await
                                    .map_err(|e| format!("Task panicked: {}", e));

                                match result {
                                    Ok(config_result) => {
                                        let error = if config_result.has_errors() {
                                            Some("Some configurations failed".to_string())
                                        } else {
                                            None
                                        };
                                        let _ = cmd_tx.send(Event::SlashCommandResult {
                                            session_id,
                                            command: "config-dangerous".to_string(),
                                            summary: config_result.summary(),
                                            error,
                                        });
                                    }
                                    Err(e) => {
                                        let _ = cmd_tx.send(Event::SlashCommandResult {
                                            session_id,
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
                let no_worktree_flag = cli.no_worktree;
                let custom_worktree_dir = cli.worktree_dir.clone();
                let custom_worktree_branch = cli.worktree_branch.clone();

                let new_init_handle = tokio::spawn(async move {
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

                    let mut state = State::new(&feature_name, &objective, max_iter)?;

                    // Set up git worktree if not disabled
                    let effective_working_dir = if no_worktree_flag {
                        let _ = tx.send(Event::SessionOutput {
                            session_id,
                            line: "[planning] Worktree disabled via --no-worktree".to_string(),
                        });
                        wd.clone()
                    } else {
                        // Get session directory for worktree
                        let session_dir = match crate::planning_paths::session_dir(&state.workflow_session_id) {
                            Ok(dir) => dir,
                            Err(e) => {
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!("[planning] Warning: Could not get session directory: {}", e),
                                });
                                return Ok::<_, anyhow::Error>((state, state_path, feature_name, wd.clone()));
                            }
                        };

                        let worktree_base = custom_worktree_dir
                            .as_ref()
                            .map(|d| d.to_path_buf())
                            .unwrap_or(session_dir);

                        match crate::git_worktree::create_session_worktree(
                            &wd,
                            &state.workflow_session_id,
                            &feature_name,
                            &worktree_base,
                            custom_worktree_branch.as_deref(),
                        ) {
                            crate::git_worktree::WorktreeSetupResult::Created(info) => {
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!("[planning] Created git worktree at: {}", info.worktree_path.display()),
                                });
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!("[planning] Working on branch: {}", info.branch_name),
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
                                        line: "[planning] Warning: Repository has submodules".to_string(),
                                    });
                                }
                                let wt_state = crate::state::WorktreeState {
                                    worktree_path: info.worktree_path.clone(),
                                    branch_name: info.branch_name,
                                    source_branch: info.source_branch,
                                    original_dir: info.original_dir,
                                };
                                state.worktree_info = Some(wt_state);
                                info.worktree_path
                            }
                            crate::git_worktree::WorktreeSetupResult::NotAGitRepo => {
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: "[planning] Not a git repository, using original directory".to_string(),
                                });
                                wd.clone()
                            }
                            crate::git_worktree::WorktreeSetupResult::Failed(err) => {
                                let _ = tx.send(Event::SessionOutput {
                                    session_id,
                                    line: format!("[planning] Warning: Git worktree setup failed: {}", err),
                                });
                                wd.clone()
                            }
                        }
                    };

                    // Pre-create plan folder and files (in ~/.planning-agent/sessions/)
                    pre_create_plan_files_with_working_dir(&state, Some(&effective_working_dir))
                        .context("Failed to pre-create plan files")?;

                    state.set_updated_at();
                    state.save(&state_path)?;

                    let _ = tx.send(Event::SessionStateUpdate {
                        session_id,
                        state: state.clone(),
                    });

                    Ok::<_, anyhow::Error>((state, state_path, feature_name, effective_working_dir))
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

fn handle_tab_switching(key: crossterm::event::KeyEvent, tab_manager: &mut TabManager) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Char('+'), m) if m.contains(KeyModifiers::CONTROL) => {
            tab_manager.add_session();
            tab_manager.active_mut().input_mode = InputMode::NamingTab;
            true
        }
        (KeyCode::Char('='), m) if m.contains(KeyModifiers::CONTROL) => {
            tab_manager.add_session();
            tab_manager.active_mut().input_mode = InputMode::NamingTab;
            true
        }
        (KeyCode::PageDown, m) if m.contains(KeyModifiers::CONTROL) => {
            tab_manager.next_tab();
            true
        }
        (KeyCode::PageUp, m) if m.contains(KeyModifiers::CONTROL) => {
            tab_manager.prev_tab();
            true
        }
        (KeyCode::Right, m) if m.contains(KeyModifiers::ALT) => {
            tab_manager.next_tab();
            true
        }
        (KeyCode::Left, m) if m.contains(KeyModifiers::ALT) => {
            tab_manager.prev_tab();
            true
        }
        (KeyCode::Char(c @ '1'..='9'), m) if m.contains(KeyModifiers::ALT) => {
            let index = (c as usize) - ('1' as usize);
            tab_manager.switch_to_tab(index);
            true
        }
        (KeyCode::Char('w'), m) if m.contains(KeyModifiers::CONTROL) => {
            tab_manager.close_tab(tab_manager.active_tab);
            true
        }
        _ => false,
    }
}

async fn handle_approval_mode_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
    workflow_config: &WorkflowConfig,
    file_index: &FileIndex,
) -> Result<bool> {
    match session.approval_mode {
        ApprovalMode::AwaitingChoice => {
            handle_awaiting_choice_input(key, session, terminal, working_dir, output_tx, workflow_config).await
        }
        ApprovalMode::EnteringFeedback => handle_entering_feedback_input(key, session, file_index).await,
        ApprovalMode::None => handle_none_mode_input(key, session),
    }
}

fn handle_none_mode_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    // Check visibility of Todos panel for focus handling
    let todos_visible = is_todo_panel_visible(session);

    // Reset focus if currently on invisible Todos panel
    session.reset_focus_if_todos_invisible(todos_visible);

    match key.code {
        KeyCode::Char('q') => {
            return Ok(true);
        }
        KeyCode::Esc => {
            // Escape: Start interrupt feedback mode if workflow is running, otherwise quit
            if session.running && session.workflow_control_tx.is_some() {
                session.start_feedback_input_for(FeedbackTarget::WorkflowInterrupt);
            } else {
                return Ok(true);
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if session.running && session.workflow_control_tx.is_some() {
                session.add_output("[planning] Stopping workflow...".to_string());
                let _ = session.workflow_control_tx.as_ref().unwrap().try_send(WorkflowCommand::Stop);
            }
        }
        KeyCode::Tab => {
            session.toggle_focus_with_visibility(todos_visible);
        }
        KeyCode::Char('j') | KeyCode::Down => match session.focused_panel {
            FocusedPanel::Output => session.scroll_down(),
            FocusedPanel::Todos => {
                let max_scroll = compute_todo_panel_max_scroll(session);
                session.todo_scroll_down(max_scroll);
            }
            FocusedPanel::Chat => session.chat_scroll_down(),
            FocusedPanel::Summary => session.summary_scroll_down(),
            FocusedPanel::Implementation => {} // Handled by ImplementationTerminal mode
        },
        KeyCode::Char('k') | KeyCode::Up => match session.focused_panel {
            FocusedPanel::Output => session.scroll_up(),
            FocusedPanel::Todos => session.todo_scroll_up(),
            FocusedPanel::Chat => session.chat_scroll_up(),
            FocusedPanel::Summary => session.summary_scroll_up(),
            FocusedPanel::Implementation => {} // Handled by ImplementationTerminal mode
        },
        KeyCode::Char('g') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_top(),
            FocusedPanel::Todos => session.todo_scroll_to_top(),
            FocusedPanel::Chat => {
                session.chat_follow_mode = false;
                if let Some(tab) = session.run_tabs.get_mut(session.active_run_tab) {
                    tab.scroll_position = 0;
                }
            }
            FocusedPanel::Summary => session.summary_scroll_to_top(),
            FocusedPanel::Implementation => {} // Handled by ImplementationTerminal mode
        },
        KeyCode::Char('G') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_bottom(),
            FocusedPanel::Todos => {
                let max_scroll = compute_todo_panel_max_scroll(session);
                session.todo_scroll_to_bottom(max_scroll);
            }
            FocusedPanel::Chat => session.chat_scroll_to_bottom(),
            FocusedPanel::Summary => {
                let max_scroll = session
                    .run_tabs
                    .get(session.active_run_tab)
                    .map(|tab| compute_run_tab_summary_max_scroll(&tab.summary_text))
                    .unwrap_or(0);
                session.summary_scroll_to_bottom(max_scroll);
            }
            FocusedPanel::Implementation => {} // Handled by ImplementationTerminal mode
        },
        KeyCode::Left => {
            if session.focused_panel == FocusedPanel::Chat
                || session.focused_panel == FocusedPanel::Summary
            {
                session.prev_run_tab();
            }
        }
        KeyCode::Right => {
            if session.focused_panel == FocusedPanel::Chat
                || session.focused_panel == FocusedPanel::Summary
            {
                session.next_run_tab();
            }
        }
        _ => {}
    }
    Ok(false)
}
