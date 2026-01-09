//! Approval-related input handling for the TUI.

use crate::tui::file_index::FileIndex;
use crate::tui::mention::update_mention_state;
use crate::tui::{
    ApprovalContext, ApprovalMode, Event, FeedbackTarget, Session, SessionStatus,
    UserApprovalResponse, WorkflowCommand,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::implementation_input::start_implementation_terminal;

/// Compute the max scroll for the plan summary popup based on wrapped lines and terminal size.
pub fn compute_plan_summary_max_scroll(plan_summary: &str) -> usize {
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let (inner_width, visible_height) =
        crate::tui::ui::util::compute_popup_summary_inner_size(term_width, term_height);

    use crate::tui::ui::util::parse_markdown_line;
    use ratatui::text::Line;
    let summary_lines: Vec<Line> = plan_summary.lines().map(parse_markdown_line).collect();
    let total_lines =
        crate::tui::ui::util::compute_wrapped_line_count(&summary_lines, inner_width);

    total_lines.saturating_sub(visible_height as usize)
}

pub async fn handle_awaiting_choice_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    match session.approval_context {
        ApprovalContext::PlanApproval => {
            handle_plan_approval_input(key, session, terminal, working_dir, output_tx).await
        }
        ApprovalContext::ReviewDecision => handle_review_decision_input(key, session).await,
        ApprovalContext::PlanGenerationFailed => {
            handle_plan_generation_failed_input(key, session).await
        }
        ApprovalContext::MaxIterationsReached => handle_max_iterations_input(key, session).await,
        ApprovalContext::UserOverrideApproval => {
            handle_user_override_input(key, session, terminal, working_dir, output_tx).await
        }
    }
}

pub async fn handle_plan_approval_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    _terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if let Some(tx) = session.approval_tx.take() {
                let _ = tx.send(UserApprovalResponse::Accept).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Complete;
        }
        KeyCode::Char('i') | KeyCode::Char('I') => {
            let plan_path = session
                .workflow_state
                .as_ref()
                .map(|s| working_dir.join(&s.plan_file))
                .unwrap_or_default();

            session.approval_tx.take();
            session.approval_mode = ApprovalMode::None;

            if let Err(e) = start_implementation_terminal(session, plan_path, output_tx) {
                session.handle_error(&format!("Failed to start implementation: {}", e));
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            session.start_feedback_input();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = compute_plan_summary_max_scroll(&session.plan_summary);
            session.scroll_summary_down(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.scroll_summary_up();
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

pub async fn handle_review_decision_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::ReviewContinue).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::ReviewRetry).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = compute_plan_summary_max_scroll(&session.plan_summary);
            session.scroll_summary_down(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.scroll_summary_up();
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

pub async fn handle_plan_generation_failed_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('r') | KeyCode::Char('R') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::PlanGenerationRetry).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::PlanGenerationContinue).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Esc => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::AbortWorkflow).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Error;
            session.error_state = Some("Plan generation failed".to_string());
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = compute_plan_summary_max_scroll(&session.plan_summary);
            session.scroll_summary_down(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.scroll_summary_up();
        }
        KeyCode::Char('q') => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

pub async fn handle_max_iterations_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('p') | KeyCode::Char('P') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::ProceedWithoutApproval).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::ContinueReviewing).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Planning;
            session.approval_context = ApprovalContext::PlanApproval;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            session.start_feedback_input();
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if let Some(tx) = session.approval_tx.clone() {
                let _ = tx.send(UserApprovalResponse::AbortWorkflow).await;
            }
            session.approval_mode = ApprovalMode::None;
            session.status = SessionStatus::Error;
            session.error_state = Some("Aborted at max iterations".to_string());
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = compute_plan_summary_max_scroll(&session.plan_summary);
            session.scroll_summary_down(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.scroll_summary_up();
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

pub async fn handle_user_override_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    _terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('i') | KeyCode::Char('I') => {
            let plan_path = session
                .workflow_state
                .as_ref()
                .map(|s| working_dir.join(&s.plan_file))
                .unwrap_or_default();

            session.approval_tx.take();
            session.approval_mode = ApprovalMode::None;

            if let Err(e) = start_implementation_terminal(session, plan_path, output_tx) {
                session.handle_error(&format!("Failed to start implementation: {}", e));
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            session.start_feedback_input();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = compute_plan_summary_max_scroll(&session.plan_summary);
            session.scroll_summary_down(max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.scroll_summary_up();
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

pub async fn handle_entering_feedback_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    file_index: &FileIndex,
) -> Result<bool> {
    // Handle @-mention dropdown navigation when active
    if session.feedback_mention_state.active && !session.feedback_mention_state.matches.is_empty() {
        match key.code {
            KeyCode::Up | KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.feedback_mention_state.select_prev();
                return Ok(false);
            }
            KeyCode::Down | KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                session.feedback_mention_state.select_next();
                return Ok(false);
            }
            KeyCode::Tab => {
                session.accept_feedback_mention();
                update_mention_state(
                    &mut session.feedback_mention_state,
                    &session.user_feedback,
                    session.cursor_position,
                    file_index,
                );
                return Ok(false);
            }
            KeyCode::Esc => {
                // If mention is active, Esc closes the dropdown instead of exiting feedback mode
                session.feedback_mention_state.clear();
                return Ok(false);
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            session.insert_feedback_newline();
        }
        KeyCode::Enter if session.last_key_was_backslash => {
            session.delete_char();
            session.insert_feedback_newline();
            session.last_key_was_backslash = false;
        }
        KeyCode::Enter => {
            let has_content =
                !session.user_feedback.trim().is_empty() || session.has_feedback_pastes();
            if has_content {
                let feedback = session.get_submit_text_feedback();

                match session.feedback_target {
                    FeedbackTarget::ApprovalDecline => {
                        // Existing behavior: decline with feedback via approval channel
                        if let Some(tx) = session.approval_tx.take() {
                            let _ = tx.send(UserApprovalResponse::Decline(feedback)).await;
                        }
                    }
                    FeedbackTarget::WorkflowInterrupt => {
                        // New behavior: send interrupt command via control channel
                        if let Some(tx) = session.workflow_control_tx.as_ref() {
                            let _ = tx.send(WorkflowCommand::Interrupt { feedback }).await;
                        }
                    }
                }

                session.user_feedback.clear();
                session.cursor_position = 0;
                session.feedback_scroll = 0;
                session.clear_feedback_pastes();
                session.approval_mode = ApprovalMode::None;
                session.feedback_target = FeedbackTarget::default();
                session.status = SessionStatus::Planning;
            }
        }
        KeyCode::Esc => {
            session.user_feedback.clear();
            session.cursor_position = 0;
            session.feedback_scroll = 0;
            session.clear_feedback_pastes();
            session.approval_mode = ApprovalMode::AwaitingChoice;
            session.feedback_target = FeedbackTarget::default();
        }
        KeyCode::Char('\\') => {
            session.insert_char('\\');
            session.last_key_was_backslash = true;
        }
        KeyCode::Char(c) => {
            session.insert_char(c);
            session.last_key_was_backslash = false;
        }
        KeyCode::Backspace => {
            session.delete_char();
        }
        KeyCode::Delete => {
            session.delete_char();
        }
        KeyCode::Left => {
            session.move_cursor_left();
        }
        KeyCode::Right => {
            session.move_cursor_right();
        }
        KeyCode::Home => {
            session.cursor_position = 0;
        }
        KeyCode::End => {
            session.cursor_position = session.user_feedback.len();
        }
        _ => {}
    }

    // Update @-mention state after any input change
    update_mention_state(
        &mut session.feedback_mention_state,
        &session.user_feedback,
        session.cursor_position,
        file_index,
    );

    Ok(false)
}
