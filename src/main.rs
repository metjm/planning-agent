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
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tui::{App, ApprovalMode, Event, EventHandler, UserApprovalResponse};

fn get_run_id() -> String {
    use std::sync::OnceLock;
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(|| {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    }).clone()
}

fn log_workflow(working_dir: &PathBuf, message: &str) {
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

async fn extract_feature_name(objective: &str, output_tx: Option<&mpsc::UnboundedSender<Event>>) -> Result<String> {
    use std::process::Stdio;
    use tokio::process::Command;

    if let Some(tx) = output_tx {
        let _ = tx.send(Event::Output("[planning] Extracting feature name...".to_string()));
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

async fn summarize_plan(plan_path: &std::path::Path, output_tx: &mpsc::UnboundedSender<Event>) -> Result<String> {
    use claude::ClaudeInvocation;

    let _ = output_tx.send(Event::Output("[planning] Generating plan summary...".to_string()));

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

    let result = ClaudeInvocation::new(prompt)
        .with_max_turns(1)
        .execute()
        .await?;

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

    // Create app and event handler
    let mut app = App::new();
    debug_log(start, "app created");
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
            let _ = init_tx.send(Event::Output(format!("[planning] Loading existing workflow: {}", feature_name)));
            State::load(&state_path)?
        } else {
            let _ = init_tx.send(Event::Output(format!("[planning] Starting new workflow: {}", feature_name)));
            let _ = init_tx.send(Event::Output(format!("[planning] Objective: {}", init_objective)));
            State::new(&feature_name, &init_objective, init_max_iterations)
        };

        // Create docs/plans directory
        let plans_dir = init_working_dir.join("docs/plans");
        std::fs::create_dir_all(&plans_dir).context("Failed to create docs/plans directory")?;

        // Save initial state
        state.save(&state_path)?;

        let _ = init_tx.send(Event::StateUpdate(state.clone()));

        Ok::<_, anyhow::Error>((state, state_path))
    });
    debug_log(start, "init task spawned");

    // Track initialization state
    let mut init_handle = Some(init_handle);
    let mut workflow_handle: Option<tokio::task::JoinHandle<Result<WorkflowResult>>> = None;
    let mut approval_tx: Option<mpsc::Sender<UserApprovalResponse>> = None;
    let mut current_state: Option<State> = None;
    let mut workflow_state_path: Option<PathBuf> = None;

    debug_log(start, "entering main loop");

    // Main event loop
    loop {
        // Draw UI
        terminal.draw(|frame| tui::ui::draw(frame, &app))?;

        // Handle events
        match event_handler.next().await? {
            Event::Key(key) => {
                // Handle approval mode input
                match app.approval_mode {
                    ApprovalMode::AwaitingChoice => {
                        match key.code {
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                // Accept
                                if let Some(tx) = approval_tx.take() {
                                    let _ = tx.send(UserApprovalResponse::Accept).await;
                                }
                                app.approval_mode = ApprovalMode::None;
                                app.should_quit = true;
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                // Decline - switch to feedback input mode
                                app.start_feedback_input();
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                // Scroll summary down
                                let max_scroll = app.plan_summary.lines().count().saturating_sub(10);
                                app.scroll_summary_down(max_scroll);
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                // Scroll summary up
                                app.scroll_summary_up();
                            }
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.should_quit = true;
                            }
                            _ => {}
                        }
                    }
                    ApprovalMode::EnteringFeedback => {
                        match key.code {
                            KeyCode::Enter => {
                                // Submit feedback
                                if !app.user_feedback.trim().is_empty() {
                                    let feedback = app.user_feedback.clone();
                                    if let Some(tx) = approval_tx.take() {
                                        let _ = tx.send(UserApprovalResponse::Decline(feedback)).await;
                                    }
                                    app.approval_mode = ApprovalMode::None;
                                }
                            }
                            KeyCode::Esc => {
                                // Cancel - go back to choice
                                app.approval_mode = ApprovalMode::AwaitingChoice;
                                app.user_feedback.clear();
                                app.cursor_position = 0;
                            }
                            KeyCode::Backspace => {
                                app.delete_char();
                            }
                            KeyCode::Left => {
                                app.move_cursor_left();
                            }
                            KeyCode::Right => {
                                app.move_cursor_right();
                            }
                            KeyCode::Char(c) => {
                                app.insert_char(c);
                            }
                            _ => {}
                        }
                    }
                    ApprovalMode::None => {
                        // Normal mode
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.should_quit = true;
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.should_quit = true;
                            }
                            KeyCode::Char('j') | KeyCode::Down => {
                                app.scroll_down();
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                app.scroll_up();
                            }
                            KeyCode::Char('g') => {
                                app.scroll_to_top();
                            }
                            KeyCode::Char('G') => {
                                app.scroll_to_bottom();
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::Tick => {
                // Update elapsed time display (automatic via app.elapsed())
            }
            Event::Output(line) => {
                // Check for cost updates before consuming line
                if line.contains("Cost: $") {
                    if let Some(cost_str) = line.split("$").nth(1) {
                        if let Ok(cost) = cost_str.trim().parse::<f64>() {
                            app.total_cost += cost;
                        }
                    }
                }
                app.add_output(line);
            }
            Event::Streaming(line) => {
                app.add_streaming(line);
            }
            Event::ToolStarted(name) => {
                app.tool_started(name);
                app.tool_call_count += 1;
            }
            Event::ToolFinished(_id) => {
                // Remove the oldest tool (FIFO) since we don't track IDs
                if !app.active_tools.is_empty() {
                    app.active_tools.remove(0);
                }
            }
            Event::StateUpdate(new_state) => {
                app.workflow_state = Some(new_state);
            }
            Event::RequestUserApproval(summary) => {
                app.start_approval(summary);
            }
            Event::BytesReceived(bytes) => {
                app.add_bytes(bytes);
            }
            Event::TokenUsage(usage) => {
                app.add_token_usage(&usage);
            }
            Event::PhaseStarted(phase) => {
                app.start_phase(phase);
            }
            Event::ToolOutput { tool_name, lines } => {
                app.add_tool_output_lines(tool_name, lines);
            }
        }

        if app.should_quit {
            break;
        }

        // Check if initialization completed - start workflow
        if let Some(handle) = init_handle.take() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok((state, state_path))) => {
                        current_state = Some(state.clone());
                        workflow_state_path = Some(state_path.clone());
                        app.workflow_state = Some(state.clone());

                        // Create approval channel and start workflow
                        let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
                        approval_tx = Some(new_approval_tx);

                        workflow_handle = Some(tokio::spawn({
                            let working_dir = working_dir.clone();
                            let tx = output_tx.clone();
                            async move {
                                run_workflow(state, working_dir, state_path, tx, new_approval_rx).await
                            }
                        }));
                    }
                    Ok(Err(e)) => {
                        app.add_output(format!("[error] Initialization failed: {}", e));
                        app.running = false;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                    Err(e) => {
                        app.add_output(format!("[error] Initialization panicked: {}", e));
                        app.running = false;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                }
            } else {
                // Put it back if not finished
                init_handle = Some(handle);
            }
        }

        // Check if workflow completed
        if let Some(handle) = workflow_handle.take() {
            if handle.is_finished() {
                match handle.await {
                    Ok(Ok(WorkflowResult::Accepted)) => {
                        app.running = false;
                        break;
                    }
                    Ok(Ok(WorkflowResult::NeedsRestart { user_feedback })) => {
                        // Restart the workflow with updated objective
                        let _ = output_tx.send(Event::Output("".to_string()));
                        let _ = output_tx.send(Event::Output("=== RESTARTING WITH YOUR FEEDBACK ===".to_string()));
                        let _ = output_tx.send(Event::Output(format!("Changes requested: {}", user_feedback)));

                        if let (Some(ref mut state), Some(ref state_path)) = (&mut current_state, &workflow_state_path) {
                            // Reset state for new iteration
                            state.phase = Phase::Planning;
                            state.iteration = 1;
                            // Append user feedback to objective
                            state.objective = format!(
                                "{}\n\nUSER FEEDBACK: The previous plan was reviewed and needs changes:\n{}",
                                state.objective,
                                user_feedback
                            );
                            state.save(state_path)?;
                            app.workflow_state = Some(state.clone());
                            app.streaming_lines.clear();

                            // Create new approval channel
                            let (new_approval_tx, new_approval_rx) = mpsc::channel::<UserApprovalResponse>(1);
                            approval_tx = Some(new_approval_tx);

                            // Spawn new workflow
                            workflow_handle = Some(tokio::spawn({
                                let state = state.clone();
                                let working_dir = working_dir.clone();
                                let state_path = state_path.clone();
                                let tx = output_tx.clone();
                                async move {
                                    run_workflow(state, working_dir, state_path, tx, new_approval_rx).await
                                }
                            }));
                        }
                    }
                    Ok(Err(e)) => {
                        app.add_output(format!("[error] Workflow failed: {}", e));
                        app.running = false;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                    Err(e) => {
                        app.add_output(format!("[error] Workflow panicked: {}", e));
                        app.running = false;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        break;
                    }
                }
            } else {
                // Put it back if not finished
                workflow_handle = Some(handle);
            }
        }
    }

    // Restore terminal
    restore_terminal(&mut terminal)?;

    Ok(())
}

fn restore_terminal(terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>) -> Result<()> {
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
    NeedsRestart { user_feedback: String },
}

async fn run_workflow(
    mut state: State,
    working_dir: PathBuf,
    state_path: PathBuf,
    output_tx: mpsc::UnboundedSender<Event>,
    mut approval_rx: mpsc::Receiver<UserApprovalResponse>,
) -> Result<WorkflowResult> {
    log_workflow(&working_dir, &format!("=== WORKFLOW START: {} ===", state.feature_name));
    log_workflow(&working_dir, &format!("Initial phase: {:?}, iteration: {}", state.phase, state.iteration));

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                log_workflow(&working_dir, ">>> ENTERING Planning phase");
                let _ = output_tx.send(Event::PhaseStarted("Planning".to_string()));
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output("=== PLANNING PHASE ===".to_string()));
                let _ = output_tx.send(Event::Output(format!("Feature: {}", state.feature_name)));
                let _ = output_tx.send(Event::Output(format!("Plan file: {}", state.plan_file.display())));

                log_workflow(&working_dir, "Calling run_planning_phase...");
                run_planning_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_planning_phase completed");

                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    log_workflow(&working_dir, "ERROR: Plan file was not created!");
                    let _ = output_tx.send(Event::Output("[error] Plan file was not created!".to_string()));
                    anyhow::bail!("Plan file not created");
                }

                log_workflow(&working_dir, "Transitioning: Planning -> Reviewing");
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                let _ = output_tx.send(Event::StateUpdate(state.clone()));
                let _ = output_tx.send(Event::Output("[planning] Transitioning to review phase...".to_string()));
            }

            Phase::Reviewing => {
                log_workflow(&working_dir, &format!(">>> ENTERING Reviewing phase (iteration {})", state.iteration));
                let _ = output_tx.send(Event::PhaseStarted("Reviewing".to_string()));
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output(format!("=== REVIEW PHASE (Iteration {}) ===", state.iteration)));

                log_workflow(&working_dir, "Calling run_review_phase...");
                run_review_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_review_phase completed");

                let feedback_path = working_dir.join(&state.feedback_file);
                if !feedback_path.exists() {
                    log_workflow(&working_dir, "ERROR: Feedback file was not created!");
                    let _ = output_tx.send(Event::Output("[error] Feedback file was not created!".to_string()));
                    anyhow::bail!("Feedback file not created");
                }

                let status = parse_feedback_status(&feedback_path)?;
                log_workflow(&working_dir, &format!("Feedback status: {:?}", status));
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        log_workflow(&working_dir, "Plan APPROVED! Transitioning to Complete");
                        let _ = output_tx.send(Event::Output("[planning] Plan APPROVED!".to_string()));
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        let _ = output_tx.send(Event::Output("[planning] Plan needs revision".to_string()));
                        if state.iteration >= state.max_iterations {
                            log_workflow(&working_dir, "Max iterations reached, stopping");
                            let _ = output_tx.send(Event::Output("[planning] Max iterations reached".to_string()));
                            break;
                        }
                        log_workflow(&working_dir, "Transitioning: Reviewing -> Revising");
                        state.transition(Phase::Revising)?;
                    }
                }
                state.save(&state_path)?;
                let _ = output_tx.send(Event::StateUpdate(state.clone()));
            }

            Phase::Revising => {
                log_workflow(&working_dir, &format!(">>> ENTERING Revising phase (iteration {})", state.iteration));
                let _ = output_tx.send(Event::PhaseStarted("Revising".to_string()));
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output(format!("=== REVISION PHASE (Iteration {}) ===", state.iteration)));

                log_workflow(&working_dir, "Calling run_revision_phase...");
                run_revision_phase(&state, &working_dir, output_tx.clone()).await?;
                log_workflow(&working_dir, "run_revision_phase completed");

                // Delete the feedback file so next review starts fresh
                let feedback_path = working_dir.join(&state.feedback_file);
                if feedback_path.exists() {
                    if let Err(e) = std::fs::remove_file(&feedback_path) {
                        log_workflow(&working_dir, &format!("Warning: Failed to delete feedback file: {}", e));
                    } else {
                        log_workflow(&working_dir, "Deleted old feedback file");
                    }
                }

                state.iteration += 1;
                log_workflow(&working_dir, &format!("Transitioning: Revising -> Reviewing (iteration now {})", state.iteration));
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                let _ = output_tx.send(Event::StateUpdate(state.clone()));
                let _ = output_tx.send(Event::Output("[planning] Transitioning to review phase...".to_string()));
            }

            Phase::Complete => {
                // This shouldn't be reached since should_continue() returns false for Complete
                // User approval logic is handled after the loop
                break;
            }
        }
    }

    log_workflow(&working_dir, &format!("=== WORKFLOW END: phase={:?}, iteration={} ===", state.phase, state.iteration));

    // If plan was approved, request user approval before finishing
    if state.phase == Phase::Complete {
        log_workflow(&working_dir, ">>> Plan complete - requesting user approval");

        // Generate plan summary
        let plan_path = working_dir.join(&state.plan_file);
        let summary = match summarize_plan(&plan_path, &output_tx).await {
            Ok(s) => s,
            Err(e) => {
                log_workflow(&working_dir, &format!("Failed to summarize plan: {}", e));
                format!("(Could not generate summary: {})\n\nThe plan has been approved by AI review.", e)
            }
        };

        let _ = output_tx.send(Event::Output("".to_string()));
        let _ = output_tx.send(Event::Output("=== PLAN APPROVED BY AI ===".to_string()));
        let _ = output_tx.send(Event::Output(format!("Completed after {} iteration(s)", state.iteration)));
        let _ = output_tx.send(Event::Output("Waiting for your approval...".to_string()));

        // Request user approval
        let _ = output_tx.send(Event::RequestUserApproval(summary));

        // Wait for user response
        log_workflow(&working_dir, "Waiting for user approval response...");
        match approval_rx.recv().await {
            Some(UserApprovalResponse::Accept) => {
                log_workflow(&working_dir, "User ACCEPTED the plan");
                let _ = output_tx.send(Event::Output("[planning] User accepted the plan!".to_string()));
                return Ok(WorkflowResult::Accepted);
            }
            Some(UserApprovalResponse::Decline(feedback)) => {
                log_workflow(&working_dir, &format!("User DECLINED with feedback: {}", feedback));
                let _ = output_tx.send(Event::Output(format!("[planning] User requested changes: {}", feedback)));
                return Ok(WorkflowResult::NeedsRestart { user_feedback: feedback });
            }
            None => {
                log_workflow(&working_dir, "Approval channel closed - treating as accept");
                return Ok(WorkflowResult::Accepted);
            }
        }
    }

    // Max iterations reached without approval
    let _ = output_tx.send(Event::Output("".to_string()));
    let _ = output_tx.send(Event::Output("=== WORKFLOW COMPLETE ===".to_string()));
    let _ = output_tx.send(Event::Output("Max iterations reached. Manual review recommended.".to_string()));

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
                    eprintln!("[state] phase={:?} iteration={}", state.phase, state.iteration);
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
        eprintln!("Plan file: {}", working_dir.join(&state.plan_file).display());
    } else {
        eprintln!("Max iterations reached. Manual review recommended.");
    }

    Ok(())
}
