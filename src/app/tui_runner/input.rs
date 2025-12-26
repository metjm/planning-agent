
use crate::app::cli::Cli;
use crate::app::headless::extract_feature_name;
use crate::config::WorkflowConfig;
use crate::state::State;
use crate::tui::{
    ApprovalContext, ApprovalMode, Event, FocusedPanel, InputMode, Session, SessionStatus,
    TabManager, UserApprovalResponse,
};
use crate::update;
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::{restore_terminal, InitHandle};

#[allow(clippy::too_many_arguments)]
pub async fn handle_key_event(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &PathBuf,
    cli: &Cli,
    _workflow_config: &WorkflowConfig,
    init_handle: &mut InitHandle,
) -> Result<bool> {
    #[allow(unused_assignments)]
    let mut should_quit = false;

    let update_in_progress = tab_manager.update_in_progress;

    if tab_manager.active().input_mode == InputMode::NamingTab {
        tab_manager.update_notice = None;
    }

    let session = tab_manager.active_mut();

    if session.error_state.is_some() {
        match key.code {
            KeyCode::Esc => {
                session.clear_error();
                return Ok(false);
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                tab_manager.close_tab(tab_manager.active_tab);
                return Ok(false);
            }
            _ => return Ok(false),
        }
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
    if session.approval_mode == ApprovalMode::None {
        if handle_tab_switching(key, tab_manager) {
            return Ok(false);
        }
    }

    let session = tab_manager.active_mut();
    should_quit = handle_approval_mode_input(key, session, terminal, working_dir).await?;

    Ok(should_quit)
}

async fn handle_naming_tab_input(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    output_tx: &mpsc::UnboundedSender<Event>,
    working_dir: &PathBuf,
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

    let session = tab_manager.active_mut();

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

            if input_text == "/update" {
                session.tab_input.clear();
                session.tab_input_cursor = 0;
                session.tab_input_scroll = 0;

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
                return Ok(false);
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
                let wd = working_dir.clone();
                let max_iter = cli.max_iterations;

                let new_init_handle = tokio::spawn(async move {
                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: "[planning] Initializing...".to_string(),
                    });

                    let feature_name = extract_feature_name(&objective, Some(&tx)).await?;

                    let state_path = wd.join(format!(".planning-agent/{}.json", feature_name));

                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: format!("[planning] Starting new workflow: {}", feature_name),
                    });
                    let _ = tx.send(Event::SessionOutput {
                        session_id,
                        line: format!("[planning] Objective: {}", objective),
                    });

                    let state = State::new(&feature_name, &objective, max_iter);

                    let plans_dir = wd.join("docs/plans");
                    std::fs::create_dir_all(&plans_dir)
                        .context("Failed to create docs/plans directory")?;

                    state.save(&state_path)?;

                    let _ = tx.send(Event::SessionStateUpdate {
                        session_id,
                        state: state.clone(),
                    });

                    Ok::<_, anyhow::Error>((state, state_path, feature_name))
                });

                *init_handle = Some((session_id, new_init_handle));
            }
        }
        KeyCode::Esc => {
            tab_manager.update_error = None;
            tab_manager.close_current_if_empty();
        }
        KeyCode::Char(c) => {
            session.insert_tab_input_char(c);
            session.last_key_was_backslash = c == '\\';
            tab_manager.update_error = None;
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
    working_dir: &PathBuf,
) -> Result<bool> {
    match session.approval_mode {
        ApprovalMode::AwaitingChoice => {
            handle_awaiting_choice_input(key, session, terminal, working_dir).await
        }
        ApprovalMode::EnteringFeedback => handle_entering_feedback_input(key, session).await,
        ApprovalMode::None => handle_none_mode_input(key, session),
    }
}

async fn handle_awaiting_choice_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
) -> Result<bool> {
    match session.approval_context {
        ApprovalContext::PlanApproval => {
            handle_plan_approval_input(key, session, terminal, working_dir).await
        }
        ApprovalContext::ReviewDecision => handle_review_decision_input(key, session).await,
        ApprovalContext::PlanGenerationFailed => {
            handle_plan_generation_failed_input(key, session).await
        }
        ApprovalContext::MaxIterationsReached => handle_max_iterations_input(key, session).await,
        ApprovalContext::UserOverrideApproval => {
            handle_user_override_input(key, session, terminal, working_dir).await
        }
    }
}

async fn handle_plan_approval_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
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

            launch_claude_implementation(terminal, plan_path)?;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            session.start_feedback_input();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = session.plan_summary.lines().count().saturating_sub(10);
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

async fn handle_review_decision_input(
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
            let max_scroll = session.plan_summary.lines().count().saturating_sub(10);
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

async fn handle_plan_generation_failed_input(
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
            let max_scroll = session.plan_summary.lines().count().saturating_sub(10);
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

async fn handle_max_iterations_input(
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
            let max_scroll = session.plan_summary.lines().count().saturating_sub(10);
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

async fn handle_user_override_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    working_dir: &PathBuf,
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

            launch_claude_implementation(terminal, plan_path)?;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => {
            session.start_feedback_input();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max_scroll = session.plan_summary.lines().count().saturating_sub(10);
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

async fn handle_entering_feedback_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
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
                if let Some(tx) = session.approval_tx.take() {
                    let _ = tx.send(UserApprovalResponse::Decline(feedback)).await;
                }
                session.user_feedback.clear();
                session.cursor_position = 0;
                session.clear_feedback_pastes();
                session.approval_mode = ApprovalMode::None;
            }
        }
        KeyCode::Esc => {
            session.approval_mode = ApprovalMode::AwaitingChoice;
            session.user_feedback.clear();
            session.cursor_position = 0;
            session.clear_feedback_pastes();
        }
        KeyCode::Backspace => {
            if !session.delete_paste_at_cursor_feedback() {
                session.delete_char();
            }
        }
        KeyCode::Left => {
            session.move_cursor_left();
        }
        KeyCode::Right => {
            session.move_cursor_right();
        }
        KeyCode::Char(c) => {
            session.insert_char(c);
            session.last_key_was_backslash = c == '\\';
        }
        _ => {
            session.last_key_was_backslash = false;
        }
    }
    Ok(false)
}

fn handle_none_mode_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            return Ok(true);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        KeyCode::Tab => {
            session.toggle_focus();
        }
        KeyCode::Char('j') | KeyCode::Down => match session.focused_panel {
            FocusedPanel::Output => session.scroll_down(),
            FocusedPanel::Chat => session.chat_scroll_down(),
            FocusedPanel::Summary => session.summary_scroll_down(),
        },
        KeyCode::Char('k') | KeyCode::Up => match session.focused_panel {
            FocusedPanel::Output => session.scroll_up(),
            FocusedPanel::Chat => session.chat_scroll_up(),
            FocusedPanel::Summary => session.summary_scroll_up(),
        },
        KeyCode::Char('g') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_top(),
            FocusedPanel::Chat => {
                session.chat_follow_mode = false;
                if let Some(tab) = session.run_tabs.get_mut(session.active_run_tab) {
                    tab.scroll_position = 0;
                }
            }
            FocusedPanel::Summary => session.summary_scroll_to_top(),
        },
        KeyCode::Char('G') => match session.focused_panel {
            FocusedPanel::Output => session.scroll_to_bottom(),
            FocusedPanel::Chat => session.chat_scroll_to_bottom(),
            FocusedPanel::Summary => session.summary_scroll_to_bottom(100),
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

fn launch_claude_implementation(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    plan_path: PathBuf,
) -> Result<()> {
    restore_terminal(terminal)?;

    let prompt = format!(
        "Please implement the following plan fully: {}",
        plan_path.display()
    );

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new("claude")
            .arg("--dangerously-skip-permissions")
            .arg(&prompt)
            .exec();
        eprintln!("Failed to launch Claude: {}", err);
        std::process::exit(1);
    }

    #[cfg(not(unix))]
    {
        let status = std::process::Command::new("claude")
            .arg("--dangerously-skip-permissions")
            .arg(&prompt)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();

        match status {
            Ok(s) => std::process::exit(s.code().unwrap_or(0)),
            Err(e) => {
                eprintln!("Failed to launch Claude: {}", e);
                std::process::exit(1);
            }
        }
    }
}
