mod claude;
mod phases;
mod skills;
mod state;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{KeyCode, KeyModifiers};
use phases::{run_planning_phase, run_review_phase, run_revision_phase};
use state::{parse_feedback_status, FeedbackStatus, Phase, State};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tui::{
    ApprovalMode, Event, EventHandler, InputMode, Session, SessionEventSender, SessionStatus,
    TabManager, UserApprovalResponse,
};

fn get_run_id() -> String {
    use std::sync::OnceLock;
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID
        .get_or_init(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string())
        .clone()
}

fn log_workflow(working_dir: &Path, message: &str) {
    let run_id = get_run_id();
    let log_path = working_dir.join(format!(".planning-agent/workflow-{}.log", run_id));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let timestamp = chrono::Local::now().format("%H:%M:%S");
        let _ = writeln!(f, "[{}] {}", timestamp, message);
    }
}

#[derive(Parser)]
#[command(name = "planning")]
#[command(about = "Iterative planning workflow orchestrator using Claude Code")]
#[command(version)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// The objective - what you want to plan (all arguments are joined)
    #[arg(trailing_var_arg = true, required = true)]
    objective: Vec<String>,

    /// Maximum iterations before stopping
    #[arg(short, long, default_value = "3")]
    max_iterations: u32,

    /// Continue existing workflow (requires --name)
    #[arg(short, long)]
    continue_workflow: bool,

    /// Explicit feature name (skip auto-generation)
    #[arg(short, long)]
    name: Option<String>,

    /// Working directory (defaults to current directory)
    #[arg(long)]
    working_dir: Option<PathBuf>,

    /// Run without TUI (headless mode)
    #[arg(long)]
    headless: bool,
}

fn debug_log(start: std::time::Instant, msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/planning-debug.log")
    {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{}][+{:?}] {}", now, start.elapsed(), msg);
    }
}

/// Shorten Claude model name for display
fn shorten_model_name(full_name: &str) -> String {
    if full_name.contains("opus") {
        if full_name.contains("4-5") || full_name.contains("4.5") {
            "opus-4.5".to_string()
        } else {
            "opus".to_string()
        }
    } else if full_name.contains("sonnet") {
        "sonnet".to_string()
    } else if full_name.contains("haiku") {
        "haiku".to_string()
    } else {
        // Take first two segments
        full_name.split('-').take(2).collect::<Vec<_>>().join("-")
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install bundled skills if they don't exist
    if let Err(e) = skills::install_skills_if_needed() {
        eprintln!("[planning-agent] Warning: Failed to install skills: {}", e);
    }

    let start = std::time::Instant::now();
    // Log run start with timestamp
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/planning-debug.log")
        {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "\n=== NEW RUN {} ===", now);
        }
    }
    debug_log(start, "main starting");

    let cli = Cli::parse();
    debug_log(start, "cli parsed");

    if cli.headless {
        run_headless(cli).await
    } else {
        run_tui(cli, start).await
    }
}

async fn extract_feature_name(
    objective: &str,
    output_tx: Option<&mpsc::UnboundedSender<Event>>,
) -> Result<String> {
    use std::process::Stdio;
    use tokio::process::Command;

    if let Some(tx) = output_tx {
        let _ = tx.send(Event::Output(
            "[planning] Extracting feature name...".to_string(),
        ));
    }

    let prompt = format!(
        r#"Extract a short kebab-case feature name (2-4 words, lowercase, hyphens) from this objective.
Output ONLY the feature name, nothing else.

Objective: {}

Example outputs: "sharing-permissions", "user-auth", "api-rate-limiting""#,
        objective
    );

    let output = Command::new("claude")
        .arg("-p")
        .arg(&prompt)
        .arg("--output-format")
        .arg("text")
        .arg("--dangerously-skip-permissions")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()
        .await?;

    let name = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>();

    if name.is_empty() {
        Ok("feature".to_string())
    } else {
        Ok(name)
    }
}

async fn summarize_plan(
    plan_path: &std::path::Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<String> {
    use claude::ClaudeInvocation;

    let _ = output_tx.send(Event::Output(
        "[planning] Generating plan summary...".to_string(),
    ));

    let plan_content = std::fs::read_to_string(plan_path)
        .with_context(|| format!("Failed to read plan file: {}", plan_path.display()))?;

    // Truncate if too long
    let plan_preview: String = plan_content.chars().take(8000).collect();

    let prompt = format!(
        r#"Summarize this implementation plan in markdown format.

Structure:
## Overview
One sentence describing what's being built.

## Key Changes
- Bullet points of main components/changes

## Implementation Steps
1. Numbered high-level steps

Keep it concise (under 25 lines). Use **bold** for emphasis.
Output ONLY the markdown, no preamble or code fences.

---
{}
---"#,
        plan_preview
    );

    let result = ClaudeInvocation::new(prompt).with_max_turns(50).execute().await?;

    Ok(result.result)
}

async fn run_tui(cli: Cli, start: std::time::Instant) -> Result<()> {
    debug_log(start, "run_tui starting");

    // Initialize terminal FIRST so we can show UI immediately
    crossterm::terminal::enable_raw_mode()?;
    debug_log(start, "raw mode enabled");
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    debug_log(start, "alternate screen entered");
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;
    debug_log(start, "terminal created");

    // Create tab manager and event handler
    let mut tab_manager = TabManager::new();
    debug_log(start, "tab manager created");
    let mut event_handler = EventHandler::new(Duration::from_millis(100));
    debug_log(start, "event handler created");
    let output_tx = event_handler.sender();

    let working_dir = cli
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let objective = cli.objective.join(" ");

    // Handle --continue validation
    if cli.continue_workflow && cli.name.is_none() {
        restore_terminal(&mut terminal)?;
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    }

    // Get the first session and set it up for CLI-provided objective
    let first_session = tab_manager.active_mut();
    first_session.input_mode = InputMode::Normal; // Skip naming, we have an objective
    first_session.status = SessionStatus::Planning;
    let first_session_id = first_session.id;

    // Spawn initialization task to run in background while UI renders
    let init_tx = output_tx.clone();
    let init_working_dir = working_dir.clone();
    let init_objective = objective.clone();
    let init_name = cli.name.clone();
    let init_continue = cli.continue_workflow;
    let init_max_iterations = cli.max_iterations;

    let init_handle = tokio::spawn(async move {
        let _ = init_tx.send(Event::Output("[planning] Initializing...".to_string()));

        // Get or generate feature name
        let feature_name = if let Some(name) = init_name {
            name
        } else {
            extract_feature_name(&init_objective, Some(&init_tx)).await?
        };

        let state_path = init_working_dir.join(format!(".planning-agent/{}.json", feature_name));

        let state = if init_continue {
            let _ = init_tx.send(Event::Output(format!(
                "[planning] Loading existing workflow: {}",
                feature_name
            )));
            State::load(&state_path)?
        } else {
            let _ = init_tx.send(Event::Output(format!(
                "[planning] Starting new workflow: {}",
                feature_name
            )));
            let _ = init_tx.send(Event::Output(format!(
                "[planning] Objective: {}",
                init_objective
            )));
            State::new(&feature_name, &init_objective, init_max_iterations)
        };

        // Create docs/plans directory
        let plans_dir = init_working_dir.join("docs/plans");
        std::fs::create_dir_all(&plans_dir).context("Failed to create docs/plans directory")?;

        // Save initial state
        state.save(&state_path)?;

        let _ = init_tx.send(Event::StateUpdate(state.clone()));

        Ok::<_, anyhow::Error>((state, state_path, feature_name))
    });
    debug_log(start, "init task spawned");

    // Track initialization state for first session
    let mut init_handle = Some((first_session_id, init_handle));
    let mut should_quit = false;

    debug_log(start, "entering main loop");

    // Main event loop
    loop {
        // Draw UI
        terminal.draw(|frame| tui::ui::draw(frame, &tab_manager))?;

        // Handle events
        match event_handler.next().await? {
            Event::Key(key) => {
                let session = tab_manager.active_mut();

                // Handle error state first - Esc clears error
                if session.error_state.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            session.clear_error();
                            continue;
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            tab_manager.close_tab(tab_manager.active_tab);
                            continue;
                        }
                        _ => continue,
                    }
                }

                // Handle tab naming input mode
                if session.input_mode == InputMode::NamingTab {
                    match key.code {
                        KeyCode::Enter => {
                            if !session.tab_input.trim().is_empty() {
                                // Start workflow with entered objective
                                let objective = session.tab_input.clone();
                                session.tab_input.clear();
                                session.tab_input_cursor = 0;
                                session.input_mode = InputMode::Normal;
                                session.status = SessionStatus::Planning;

                                // Spawn workflow for this new session
                                let session_id = session.id;
                                let tx = output_tx.clone();
                                let wd = working_dir.clone();
                                let max_iter = cli.max_iterations;

                                let new_init_handle = tokio::spawn(async move {
                                    let _ = tx.send(Event::SessionOutput {
                                        session_id,
                                        line: "[planning] Initializing...".to_string(),
                                    });

                                    let feature_name =
                                        extract_feature_name(&objective, Some(&tx)).await?;

                                    let state_path =
                                        wd.join(format!(".planning-agent/{}.json", feature_name));

                                    let _ = tx.send(Event::SessionOutput {
                                        session_id,
                                        line: format!(
                                            "[planning] Starting new workflow: {}",
                                            feature_name
                                        ),
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

                                init_handle = Some((session_id, new_init_handle));
                            }
                        }
                        KeyCode::Esc => {
                            // Cancel - remove empty tab
                            tab_manager.close_current_if_empty();
                        }
                        KeyCode::Char(c) => {
                            session.insert_tab_input_char(c);
                        }
                        KeyCode::Backspace => {
                            session.delete_tab_input_char();
                        }
                        KeyCode::Left => {
                            session.move_tab_input_cursor_left();
                        }
                        KeyCode::Right => {
                            session.move_tab_input_cursor_right();
                        }
                        _ => {}
                    }
                    continue;
                }

                // Handle tab switching (only when not in input/approval mode)
                if session.approval_mode == ApprovalMode::None {
                    match (key.code, key.modifiers) {
                        // Ctrl++ creates new tab
                        (KeyCode::Char('+'), m) if m.contains(KeyModifiers::CONTROL) => {
                            tab_manager.add_session();
                            tab_manager.active_mut().input_mode = InputMode::NamingTab;
                            continue;
                        }
                        // Also handle Ctrl+= (since + is shift+= on most keyboards)
                        (KeyCode::Char('='), m) if m.contains(KeyModifiers::CONTROL) => {
                            tab_manager.add_session();
                            tab_manager.active_mut().input_mode = InputMode::NamingTab;
                            continue;
                        }
                        // Ctrl+PageDown goes to next tab
                        (KeyCode::PageDown, m) if m.contains(KeyModifiers::CONTROL) => {
                            tab_manager.next_tab();
                            continue;
                        }
                        // Ctrl+PageUp goes to previous tab
                        (KeyCode::PageUp, m) if m.contains(KeyModifiers::CONTROL) => {
                            tab_manager.prev_tab();
                            continue;
                        }
                        // Alt+Right for next tab (alternative)
                        (KeyCode::Right, m) if m.contains(KeyModifiers::ALT) => {
                            tab_manager.next_tab();
                            continue;
                        }
                        // Alt+Left for previous tab (alternative)
                        (KeyCode::Left, m) if m.contains(KeyModifiers::ALT) => {
                            tab_manager.prev_tab();
                            continue;
                        }
                        // Alt+1 through Alt+9 for direct tab selection
                        (KeyCode::Char(c @ '1'..='9'), m) if m.contains(KeyModifiers::ALT) => {
                            let index = (c as usize) - ('1' as usize);
                            tab_manager.switch_to_tab(index);
                            continue;
                        }
                        // Ctrl+W closes current tab
                        (KeyCode::Char('w'), m) if m.contains(KeyModifiers::CONTROL) => {
                            tab_manager.close_tab(tab_manager.active_tab);
                            continue;
                        }
                        _ => {}
                    }
                }

                // Handle approval mode input
                match session.approval_mode {
                    ApprovalMode::AwaitingChoice => match key.code {
                        KeyCode::Char('a') | KeyCode::Char('A') => {
                            if let Some(tx) = session.approval_tx.take() {
                                let _ = tx.send(UserApprovalResponse::Accept).await;
                            }
                            session.approval_mode = ApprovalMode::None;
                            session.status = SessionStatus::Complete;
                        }
                        KeyCode::Char('i') | KeyCode::Char('I') => {
                            if let Some(tx) = session.approval_tx.take() {
                                let plan_file = session
                                    .workflow_state
                                    .as_ref()
                                    .map(|s| s.plan_file.clone())
                                    .unwrap_or_default();
                                let _ = tx.send(UserApprovalResponse::AcceptAndImplement(plan_file)).await;
                            }
                            session.approval_mode = ApprovalMode::None;
                            session.status = SessionStatus::Implementing;
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
                            should_quit = true;
                        }
                        _ => {}
                    },
                    ApprovalMode::EnteringFeedback => match key.code {
                        KeyCode::Enter => {
                            if !session.user_feedback.trim().is_empty() {
                                let feedback = session.user_feedback.clone();
                                if let Some(tx) = session.approval_tx.take() {
                                    let _ = tx.send(UserApprovalResponse::Decline(feedback)).await;
                                }
                                session.approval_mode = ApprovalMode::None;
                            }
                        }
                        KeyCode::Esc => {
                            session.approval_mode = ApprovalMode::AwaitingChoice;
                            session.user_feedback.clear();
                            session.cursor_position = 0;
                        }
                        KeyCode::Backspace => {
                            session.delete_char();
                        }
                        KeyCode::Left => {
                            session.move_cursor_left();
                        }
                        KeyCode::Right => {
                            session.move_cursor_right();
                        }
                        KeyCode::Char(c) => {
                            session.insert_char(c);
                        }
                        _ => {}
                    },
                    ApprovalMode::None => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            should_quit = true;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            should_quit = true;
                        }
                        KeyCode::Tab => {
                            session.toggle_focus();
                        }
                        KeyCode::Char('j') | KeyCode::Down => match session.focused_panel {
                            tui::FocusedPanel::Output => session.scroll_down(),
                            tui::FocusedPanel::Streaming => session.streaming_scroll_down(),
                        },
                        KeyCode::Char('k') | KeyCode::Up => match session.focused_panel {
                            tui::FocusedPanel::Output => session.scroll_up(),
                            tui::FocusedPanel::Streaming => session.streaming_scroll_up(),
                        },
                        KeyCode::Char('g') => match session.focused_panel {
                            tui::FocusedPanel::Output => session.scroll_to_top(),
                            tui::FocusedPanel::Streaming => {
                                session.streaming_follow_mode = false;
                                session.streaming_scroll_position = 0;
                            }
                        },
                        KeyCode::Char('G') => match session.focused_panel {
                            tui::FocusedPanel::Output => session.scroll_to_bottom(),
                            tui::FocusedPanel::Streaming => session.streaming_scroll_to_bottom(),
                        },
                        _ => {}
                    },
                }
            }
            Event::Tick => {
                // Update elapsed time display (automatic via session.elapsed())
            }
            Event::Resize => {
                // Terminal resize is handled automatically by ratatui
            }

            // Legacy events for backwards compatibility with first session
            Event::Output(line) => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
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
            Event::Streaming(line) => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                    session.add_streaming(line);
                }
            }
            Event::ToolStarted(name) => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                    session.tool_started(name);
                    session.tool_call_count += 1;
                }
            }
            Event::ToolFinished(_id) => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                    if !session.active_tools.is_empty() {
                        session.active_tools.remove(0);
                    }
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
            Event::ToolResultReceived { tool_id: _, is_error } => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                    if is_error {
                        session.tool_error_count += 1;
                    }
                    if !session.active_tools.is_empty() {
                        let (_, start_time) = session.active_tools.remove(0);
                        let duration_ms = start_time.elapsed().as_millis() as u64;
                        session.total_tool_duration_ms += duration_ms;
                        session.completed_tool_count += 1;
                    }
                }
            }
            Event::StopReason(reason) => {
                if let Some(session) = tab_manager.session_by_id_mut(first_session_id) {
                    session.last_stop_reason = Some(reason);
                }
            }

            // Session-routed events for multi-tab support
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
            Event::SessionTokenUsage { session_id, usage } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    session.add_token_usage(&usage);
                }
            }
            Event::SessionToolStarted { session_id, name } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    session.tool_started(name);
                    session.tool_call_count += 1;
                }
            }
            Event::SessionToolFinished { session_id, id: _ } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    if !session.active_tools.is_empty() {
                        session.active_tools.remove(0);
                    }
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
                tool_id: _,
                is_error,
            } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    if is_error {
                        session.tool_error_count += 1;
                    }
                    if !session.active_tools.is_empty() {
                        let (_, start_time) = session.active_tools.remove(0);
                        let duration_ms = start_time.elapsed().as_millis() as u64;
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
            Event::SessionImplOutput { session_id, line } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    session.add_streaming(line);
                }
            }
            Event::SessionImplComplete { session_id, success } => {
                if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                    if success {
                        session.status = SessionStatus::Complete;
                    } else {
                        session.status = SessionStatus::Error;
                    }
                    session.impl_handle = None;
                }
            }
        }

        if should_quit {
            break;
        }

        // Check if initialization completed - start workflow
        if let Some((session_id, handle)) = init_handle.take() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok((state, state_path, feature_name))) => {
                        if let Some(session) = tab_manager.session_by_id_mut(session_id) {
                            session.name = feature_name;
                            session.workflow_state = Some(state.clone());

                            // Create approval channel and start workflow
                            let (new_approval_tx, new_approval_rx) =
                                mpsc::channel::<UserApprovalResponse>(1);
                            session.approval_tx = Some(new_approval_tx);

                            let workflow_handle = tokio::spawn({
                                let working_dir = working_dir.clone();
                                let tx = output_tx.clone();
                                let sid = session_id;
                                async move {
                                    run_workflow(
                                        state,
                                        working_dir,
                                        state_path,
                                        tx,
                                        new_approval_rx,
                                        sid,
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
            } else {
                // Put it back if not finished
                init_handle = Some((session_id, handle));
            }
        }

        // Check all sessions for completed workflows
        for session in tab_manager.sessions_mut() {
            if let Some(handle) = session.workflow_handle.take() {
                if handle.is_finished() {
                    match handle.await {
                        Ok(Ok(WorkflowResult::Accepted)) => {
                            session.status = SessionStatus::Complete;
                            session.running = false;
                        }
                        Ok(Ok(WorkflowResult::AcceptAndImplement { plan_file })) => {
                            // Start implementation subprocess
                            session.status = SessionStatus::Implementing;
                            let plan_path = working_dir.join(&plan_file);

                            // Spawn implementation subprocess
                            start_implementation(session, plan_path, output_tx.clone());
                        }
                        Ok(Ok(WorkflowResult::NeedsRestart { user_feedback })) => {
                            // Restart the workflow with updated objective
                            session.add_output("".to_string());
                            session.add_output("=== RESTARTING WITH YOUR FEEDBACK ===".to_string());
                            session.add_output(format!("Changes requested: {}", user_feedback));

                            if let Some(ref mut state) = session.workflow_state {
                                // Reset state for new iteration
                                state.phase = Phase::Planning;
                                state.iteration = 1;
                                state.objective = format!(
                                    "{}\n\nUSER FEEDBACK: The previous plan was reviewed and needs changes:\n{}",
                                    state.objective,
                                    user_feedback
                                );
                                let state_path = working_dir.join(format!(
                                    ".planning-agent/{}.json",
                                    state.feature_name
                                ));
                                let _ = state.save(&state_path);
                                session.streaming_lines.clear();
                                session.status = SessionStatus::Planning;

                                // Create new approval channel
                                let (new_approval_tx, new_approval_rx) =
                                    mpsc::channel::<UserApprovalResponse>(1);
                                session.approval_tx = Some(new_approval_tx);

                                // Spawn new workflow
                                let new_handle = tokio::spawn({
                                    let state = state.clone();
                                    let working_dir = working_dir.clone();
                                    let tx = output_tx.clone();
                                    let sid = session.id;
                                    async move {
                                        run_workflow(
                                            state,
                                            working_dir,
                                            state_path,
                                            tx,
                                            new_approval_rx,
                                            sid,
                                        )
                                        .await
                                    }
                                });

                                session.workflow_handle = Some(new_handle);
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
                    // Put it back if not finished
                    session.workflow_handle = Some(handle);
                }
            }
        }
    }

    // Restore terminal
    restore_terminal(&mut terminal)?;

    Ok(())
}

/// Start implementation subprocess and stream output to session
fn start_implementation(
    session: &mut Session,
    plan_path: PathBuf,
    output_tx: mpsc::UnboundedSender<Event>,
) {
    let prompt = format!(
        "Please implement the following plan fully: {}",
        plan_path.display()
    );

    let session_id = session.id;

    // Spawn implementation subprocess
    let handle = tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new("claude");
        cmd.arg("--dangerously-skip-permissions")
            .arg(&prompt)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();

                // Stream stdout
                if let Some(stdout) = stdout {
                    let tx = output_tx.clone();
                    let reader = tokio::io::BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = tx.send(Event::SessionImplOutput {
                            session_id,
                            line,
                        });
                    }
                }

                // Stream stderr
                if let Some(stderr) = stderr {
                    let tx = output_tx.clone();
                    let reader = tokio::io::BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = tx.send(Event::SessionImplOutput {
                            session_id,
                            line: format!("[stderr] {}", line),
                        });
                    }
                }

                // Wait for completion
                let status = child.wait().await;
                let success = status.map(|s| s.success()).unwrap_or(false);
                let _ = output_tx.send(Event::SessionImplComplete { session_id, success });
            }
            Err(e) => {
                let _ = output_tx.send(Event::SessionWorkflowError {
                    session_id,
                    error: format!("Failed to start Claude: {}", e),
                });
            }
        }
    });

    // Store the child handle (though we don't really need it since we're tracking via events)
    // The handle will complete when the subprocess exits
    drop(handle);
}

fn restore_terminal(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Result from the workflow - either completed or needs restart with user feedback
pub enum WorkflowResult {
    Accepted,
    AcceptAndImplement { plan_file: PathBuf },
    NeedsRestart { user_feedback: String },
}

async fn run_workflow(
    mut state: State,
    working_dir: PathBuf,
    state_path: PathBuf,
    output_tx: mpsc::UnboundedSender<Event>,
    mut approval_rx: mpsc::Receiver<UserApprovalResponse>,
    session_id: usize,
) -> Result<WorkflowResult> {
    log_workflow(
        &working_dir,
        &format!("=== WORKFLOW START: {} ===", state.feature_name),
    );
    log_workflow(
        &working_dir,
        &format!(
            "Initial phase: {:?}, iteration: {}",
            state.phase, state.iteration
        ),
    );

    // Create session event sender for this workflow
    let sender = SessionEventSender::new(session_id, output_tx.clone());

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                log_workflow(&working_dir, ">>> ENTERING Planning phase");
                sender.send_phase_started("Planning".to_string());
                sender.send_output("".to_string());
                sender.send_output("=== PLANNING PHASE ===".to_string());
                sender.send_output(format!("Feature: {}", state.feature_name));
                sender.send_output(format!("Plan file: {}", state.plan_file.display()));

                log_workflow(&working_dir, "Calling run_planning_phase...");
                run_planning_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_planning_phase completed");

                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    log_workflow(&working_dir, "ERROR: Plan file was not created!");
                    sender.send_output("[error] Plan file was not created!".to_string());
                    anyhow::bail!("Plan file not created");
                }

                log_workflow(&working_dir, "Transitioning: Planning -> Reviewing");
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                sender.send_state_update(state.clone());
                sender.send_output("[planning] Transitioning to review phase...".to_string());
            }

            Phase::Reviewing => {
                log_workflow(
                    &working_dir,
                    &format!(
                        ">>> ENTERING Reviewing phase (iteration {})",
                        state.iteration
                    ),
                );
                sender.send_phase_started("Reviewing".to_string());
                sender.send_output("".to_string());
                sender.send_output(format!(
                    "=== REVIEW PHASE (Iteration {}) ===",
                    state.iteration
                ));

                log_workflow(&working_dir, "Calling run_review_phase...");
                run_review_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_review_phase completed");

                let feedback_path = working_dir.join(&state.feedback_file);
                if !feedback_path.exists() {
                    log_workflow(&working_dir, "ERROR: Feedback file was not created!");
                    sender.send_output("[error] Feedback file was not created!".to_string());
                    anyhow::bail!("Feedback file not created");
                }

                let status = parse_feedback_status(&feedback_path)?;
                log_workflow(&working_dir, &format!("Feedback status: {:?}", status));
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        log_workflow(&working_dir, "Plan APPROVED! Transitioning to Complete");
                        sender.send_output("[planning] Plan APPROVED!".to_string());
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        sender.send_output("[planning] Plan needs revision".to_string());
                        if state.iteration >= state.max_iterations {
                            log_workflow(&working_dir, "Max iterations reached, stopping");
                            sender.send_output("[planning] Max iterations reached".to_string());
                            break;
                        }
                        log_workflow(&working_dir, "Transitioning: Reviewing -> Revising");
                        state.transition(Phase::Revising)?;
                    }
                }
                state.save(&state_path)?;
                sender.send_state_update(state.clone());
            }

            Phase::Revising => {
                log_workflow(
                    &working_dir,
                    &format!(
                        ">>> ENTERING Revising phase (iteration {})",
                        state.iteration
                    ),
                );
                sender.send_phase_started("Revising".to_string());
                sender.send_output("".to_string());
                sender.send_output(format!(
                    "=== REVISION PHASE (Iteration {}) ===",
                    state.iteration
                ));

                log_workflow(&working_dir, "Calling run_revision_phase...");
                run_revision_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_revision_phase completed");

                // Delete the feedback file so next review starts fresh
                let feedback_path = working_dir.join(&state.feedback_file);
                if feedback_path.exists() {
                    if let Err(e) = std::fs::remove_file(&feedback_path) {
                        log_workflow(
                            &working_dir,
                            &format!("Warning: Failed to delete feedback file: {}", e),
                        );
                    } else {
                        log_workflow(&working_dir, "Deleted old feedback file");
                    }
                }

                state.iteration += 1;
                log_workflow(
                    &working_dir,
                    &format!(
                        "Transitioning: Revising -> Reviewing (iteration now {})",
                        state.iteration
                    ),
                );
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                sender.send_state_update(state.clone());
                sender.send_output("[planning] Transitioning to review phase...".to_string());
            }

            Phase::Complete => {
                break;
            }
        }
    }

    log_workflow(
        &working_dir,
        &format!(
            "=== WORKFLOW END: phase={:?}, iteration={} ===",
            state.phase, state.iteration
        ),
    );

    // If plan was approved, request user approval before finishing
    if state.phase == Phase::Complete {
        log_workflow(&working_dir, ">>> Plan complete - requesting user approval");

        // Generate plan summary
        let plan_path = working_dir.join(&state.plan_file);
        let summary = match summarize_plan(&plan_path, &output_tx).await {
            Ok(s) => s,
            Err(e) => {
                log_workflow(&working_dir, &format!("Failed to summarize plan: {}", e));
                format!(
                    "(Could not generate summary: {})\n\nThe plan has been approved by AI review.",
                    e
                )
            }
        };

        sender.send_output("".to_string());
        sender.send_output("=== PLAN APPROVED BY AI ===".to_string());
        sender.send_output(format!("Completed after {} iteration(s)", state.iteration));
        sender.send_output("Waiting for your approval...".to_string());

        // Request user approval
        sender.send_approval_request(summary);

        // Wait for user response
        log_workflow(&working_dir, "Waiting for user approval response...");
        match approval_rx.recv().await {
            Some(UserApprovalResponse::Accept) => {
                log_workflow(&working_dir, "User ACCEPTED the plan");
                sender.send_output("[planning] User accepted the plan!".to_string());
                return Ok(WorkflowResult::Accepted);
            }
            Some(UserApprovalResponse::AcceptAndImplement(plan_file)) => {
                log_workflow(&working_dir, "User chose ACCEPT AND IMPLEMENT");
                sender.send_output("[planning] Starting implementation...".to_string());
                return Ok(WorkflowResult::AcceptAndImplement { plan_file });
            }
            Some(UserApprovalResponse::Decline(feedback)) => {
                log_workflow(
                    &working_dir,
                    &format!("User DECLINED with feedback: {}", feedback),
                );
                sender.send_output(format!("[planning] User requested changes: {}", feedback));
                return Ok(WorkflowResult::NeedsRestart {
                    user_feedback: feedback,
                });
            }
            None => {
                log_workflow(&working_dir, "Approval channel closed - treating as accept");
                return Ok(WorkflowResult::Accepted);
            }
        }
    }

    // Max iterations reached without approval
    sender.send_output("".to_string());
    sender.send_output("=== WORKFLOW COMPLETE ===".to_string());
    sender.send_output("Max iterations reached. Manual review recommended.".to_string());

    Ok(WorkflowResult::Accepted)
}

async fn run_headless(cli: Cli) -> Result<()> {
    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let objective = cli.objective.join(" ");

    let feature_name = if let Some(name) = cli.name {
        name
    } else if cli.continue_workflow {
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    } else {
        eprintln!("[planning] Extracting feature name...");
        extract_feature_name(&objective, None).await?
    };

    let state_path = working_dir.join(format!(".planning-agent/{}.json", feature_name));

    let mut state = if cli.continue_workflow {
        eprintln!("[planning] Loading existing workflow: {}", feature_name);
        State::load(&state_path)?
    } else {
        eprintln!("[planning] Starting new workflow: {}", feature_name);
        eprintln!("[planning] Objective: {}", objective);
        State::new(&feature_name, &objective, cli.max_iterations)
    };

    let plans_dir = working_dir.join("docs/plans");
    std::fs::create_dir_all(&plans_dir).context("Failed to create docs/plans directory")?;

    state.save(&state_path)?;

    // Create a channel for streaming output (printed to stderr in headless mode)
    let (output_tx, mut output_rx) = mpsc::unbounded_channel::<Event>();

    // Spawn task to print streaming output to stderr
    tokio::spawn(async move {
        while let Some(event) = output_rx.recv().await {
            match event {
                Event::Output(line) | Event::Streaming(line) => {
                    eprintln!("{}", line);
                }
                Event::ToolStarted(name) => {
                    eprintln!("[tool started] {}", name);
                }
                Event::ToolFinished(id) => {
                    eprintln!("[tool finished] {}", id);
                }
                Event::StateUpdate(state) => {
                    eprintln!(
                        "[state] phase={:?} iteration={}",
                        state.phase, state.iteration
                    );
                }
                Event::TurnCompleted => {
                    eprintln!("[turn] completed");
                }
                Event::ModelDetected(name) => {
                    eprintln!("[model] {}", name);
                }
                Event::ToolResultReceived { tool_id, is_error } => {
                    if is_error {
                        eprintln!("[tool error] {}", tool_id);
                    }
                }
                Event::StopReason(reason) => {
                    eprintln!("[stop] {}", reason);
                }
                _ => {}
            }
        }
    });

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                eprintln!("\n=== PLANNING PHASE ===");
                run_planning_phase(&state, &working_dir, output_tx.clone()).await?;

                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    anyhow::bail!("Plan file not created: {}", plan_path.display());
                }

                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
            }

            Phase::Reviewing => {
                eprintln!("\n=== REVIEW PHASE (Iteration {}) ===", state.iteration);
                run_review_phase(&state, &working_dir, output_tx.clone()).await?;

                let feedback_path = working_dir.join(&state.feedback_file);
                if !feedback_path.exists() {
                    anyhow::bail!("Feedback file not created: {}", feedback_path.display());
                }

                let status = parse_feedback_status(&feedback_path)?;
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        eprintln!("[planning] Plan APPROVED!");
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        eprintln!("[planning] Plan needs revision");
                        if state.iteration >= state.max_iterations {
                            eprintln!("[planning] Max iterations reached");
                            break;
                        }
                        state.transition(Phase::Revising)?;
                    }
                }
                state.save(&state_path)?;
            }

            Phase::Revising => {
                eprintln!("\n=== REVISION PHASE (Iteration {}) ===", state.iteration);
                run_revision_phase(&state, &working_dir, output_tx.clone()).await?;

                state.iteration += 1;
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
            }

            Phase::Complete => break,
        }
    }

    eprintln!("\n=== WORKFLOW COMPLETE ===");
    if state.phase == Phase::Complete {
        eprintln!("Plan APPROVED after {} iteration(s)", state.iteration);
        eprintln!(
            "Plan file: {}",
            working_dir.join(&state.plan_file).display()
        );
    } else {
        eprintln!("Max iterations reached. Manual review recommended.");
    }

    Ok(())
}
