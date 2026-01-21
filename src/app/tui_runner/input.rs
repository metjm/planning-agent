use crate::app::cli::Cli;
use crate::config::WorkflowConfig;
use crate::phases::implementation::{run_implementation_interaction, IMPLEMENTATION_FOLLOWUP_PHASE};
use crate::tui::file_index::FileIndex;
use crate::tui::ui::util::{
    compute_summary_panel_inner_size, compute_wrapped_line_count,
    compute_wrapped_line_count_text,
};
use crate::tui::{
    ApprovalMode, Event, FeedbackTarget, FocusedPanel, InputMode, Session,
    SessionEventSender, TabManager, WorkflowCommand,
};
use anyhow::Result;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::text::Line;
use std::path::Path;
use tokio::sync::{mpsc, watch};

use super::approval_input::{handle_awaiting_choice_input, handle_entering_feedback_input};
use super::input_naming::handle_naming_tab_input;
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

/// Compute the max scroll for the review history panel.
/// The panel is shown when terminal width >= 100, taking up 30% of the chat area.
fn compute_review_history_max_scroll(session: &Session, term_width: u16, term_height: u16) -> usize {
    // The review history panel is only shown when width >= 100
    if term_width < 100 {
        return 0;
    }

    // Approximate the visible height:
    // Main layout: top bar (2), footer (3) = 5 overhead
    // Main content split: 70% left, 30% right (stats)
    // Chat area in the left 70%: 60% of main content height
    // Review history panel: 30% of chat area width
    // The panel has borders, so inner_height = panel_height - 2

    let main_content_height = term_height.saturating_sub(5);
    let chat_area_height = (main_content_height as f32 * 0.60) as u16;
    let panel_height = chat_area_height;
    let visible_height = panel_height.saturating_sub(2) as usize;

    // Count content lines: each round has a header line + reviewer lines + spacing
    let mut content_lines = 0usize;
    if session.review_history.is_empty() {
        content_lines = 1; // "No reviews yet"
    } else {
        for round in &session.review_history {
            content_lines += 1; // Round header
            content_lines += round.reviewers.len(); // Reviewer entries
            content_lines += 1; // Spacing
        }
    }

    content_lines.saturating_sub(visible_height)
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

    // Handle implementation success modal input (intercept keys before other handlers)
    if session.implementation_success_modal.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                session.close_implementation_success();
            }
            _ => {}
        }
        return Ok(false);
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

async fn handle_implementation_chat_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    output_tx: &mpsc::UnboundedSender<Event>,
    _working_dir: &Path,
) -> Result<bool> {
    if !session.can_interact_with_implementation() {
        session.add_output(
            "[implementation] Follow-up unavailable: no conversation ID or configuration".to_string(),
        );
        session.focused_panel = FocusedPanel::Chat;
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        KeyCode::Tab => {
            let todos_visible = is_todo_panel_visible(session);
            session.toggle_focus_with_visibility(todos_visible);
        }
        KeyCode::Esc => {
            if session.implementation_interaction.running {
                if let Some(tx) = session.implementation_interaction.cancel_tx.as_ref() {
                    let _ = tx.send(true);
                }
            } else {
                session.focused_panel = FocusedPanel::Chat;
            }
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
            if session.implementation_interaction.running {
                session.add_output(
                    "[implementation] Follow-up already running. Press Esc to cancel.".to_string(),
                );
                return Ok(false);
            }

            let has_content =
                !session.tab_input.trim().is_empty() || session.has_tab_input_pastes();
            if !has_content {
                return Ok(false);
            }

            let message = session.get_submit_text_tab().trim().to_string();
            if message.is_empty() {
                return Ok(false);
            }

            let Some(context) = session.context.clone() else {
                session.add_output(
                    "[implementation] Follow-up unavailable: missing session context".to_string(),
                );
                return Ok(false);
            };
            let Some(state) = session.workflow_state.clone() else {
                session.add_output(
                    "[implementation] Follow-up unavailable: missing workflow state".to_string(),
                );
                return Ok(false);
            };

            let session_id = session.id;
            let run_id = session.current_run_id;
            let (cancel_tx, cancel_rx) = watch::channel(false);

            session.implementation_interaction.running = true;
            session.implementation_interaction.cancel_tx = Some(cancel_tx);

            session.add_chat_message("user", IMPLEMENTATION_FOLLOWUP_PHASE, message.clone());
            session.tab_input.clear();
            session.tab_input_cursor = 0;
            session.tab_input_scroll = 0;
            session.last_key_was_backslash = false;
            session.clear_tab_input_pastes();
            session.tab_mention_state.clear();
            session.tab_slash_state.clear();

            let working_dir = context.effective_working_dir.clone();
            let state_path = context.state_path.clone();
            let workflow_config = context.workflow_config.clone();
            let session_sender = SessionEventSender::new(session_id, run_id, output_tx.clone());

            tokio::spawn(async move {
                let _ = run_implementation_interaction(
                    state,
                    workflow_config,
                    working_dir,
                    state_path,
                    message,
                    session_sender,
                    cancel_rx,
                )
                .await;
            });
        }
        KeyCode::Char(c) => {
            session.insert_tab_input_char(c);
            session.last_key_was_backslash = c == '\\';
        }
        KeyCode::Backspace => {
            session.last_key_was_backslash = false;
            if !session.delete_paste_at_cursor_tab() {
                session.delete_tab_input_char();
            }
        }
        KeyCode::Left => session.move_tab_input_cursor_left(),
        KeyCode::Right => session.move_tab_input_cursor_right(),
        KeyCode::Up => session.move_tab_input_cursor_up(),
        KeyCode::Down => session.move_tab_input_cursor_down(),
        _ => {}
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
    if session.approval_mode == ApprovalMode::None
        && session.focused_panel == FocusedPanel::ChatInput
    {
        return handle_implementation_chat_input(key, session, output_tx, working_dir).await;
    }

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
            if session.implementation_interaction.running {
                if let Some(tx) = session.implementation_interaction.cancel_tx.as_ref() {
                    let _ = tx.send(true);
                }
            } else if session.running && session.workflow_control_tx.is_some() {
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
        KeyCode::Char('j') | KeyCode::Down => {
            match session.focused_panel {
                FocusedPanel::Output => session.scroll_down(),
                FocusedPanel::Todos => {
                    let max_scroll = compute_todo_panel_max_scroll(session);
                    session.todo_scroll_down(max_scroll);
                }
                FocusedPanel::Chat => session.chat_scroll_down(),
                FocusedPanel::ChatInput => {}
                FocusedPanel::Summary => session.summary_scroll_down(),
                FocusedPanel::Unknown => {} // Legacy variant - no action
            }
            // Fallback: scroll review history panel if visible
            let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
            if term_width >= 100 {
                let max_scroll = compute_review_history_max_scroll(session, term_width, term_height);
                session.review_history_scroll_down(max_scroll);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            match session.focused_panel {
                FocusedPanel::Output => session.scroll_up(),
                FocusedPanel::Todos => session.todo_scroll_up(),
                FocusedPanel::Chat => session.chat_scroll_up(),
                FocusedPanel::ChatInput => {}
                FocusedPanel::Summary => session.summary_scroll_up(),
                FocusedPanel::Unknown => {} // Legacy variant - no action
            }
            // Fallback: scroll review history panel if visible
            let (term_width, _) = crossterm::terminal::size().unwrap_or((80, 24));
            if term_width >= 100 {
                session.review_history_scroll_up();
            }
        }
        KeyCode::Char('g') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_top(),
            FocusedPanel::Todos => session.todo_scroll_to_top(),
            FocusedPanel::Chat => {
                session.chat_follow_mode = false;
                if let Some(tab) = session.run_tabs.get_mut(session.active_run_tab) {
                    tab.scroll_position = 0;
                }
            }
            FocusedPanel::ChatInput => {}
            FocusedPanel::Summary => session.summary_scroll_to_top(),
            FocusedPanel::Unknown => {} // Legacy variant - no action
        },
        KeyCode::Char('G') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_bottom(),
            FocusedPanel::Todos => {
                let max_scroll = compute_todo_panel_max_scroll(session);
                session.todo_scroll_to_bottom(max_scroll);
            }
            FocusedPanel::Chat => session.chat_scroll_to_bottom(),
            FocusedPanel::ChatInput => {}
            FocusedPanel::Summary => {
                let max_scroll = session
                    .run_tabs
                    .get(session.active_run_tab)
                    .map(|tab| compute_run_tab_summary_max_scroll(&tab.summary_text))
                    .unwrap_or(0);
                session.summary_scroll_to_bottom(max_scroll);
            }
            FocusedPanel::Unknown => {} // Legacy variant - no action
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
