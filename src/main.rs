mod claude;
mod phases;
mod state;
mod tui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{KeyCode, KeyModifiers};
use phases::{run_planning_phase, run_review_phase, run_revision_phase};
use state::{parse_feedback_status, FeedbackStatus, Phase, State};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tui::{App, Event, EventHandler};

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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.headless {
        run_headless(cli).await
    } else {
        run_tui(cli).await
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

async fn run_tui(cli: Cli) -> Result<()> {
    // Initialize terminal
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Create app and event handler
    let mut app = App::new();
    let mut event_handler = EventHandler::new(Duration::from_millis(100));
    let output_tx = event_handler.sender();

    let working_dir = cli
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let objective = cli.objective.join(" ");

    // Get or generate feature name
    let feature_name = if let Some(name) = cli.name.clone() {
        name
    } else if cli.continue_workflow {
        restore_terminal(&mut terminal)?;
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    } else {
        extract_feature_name(&objective, Some(&output_tx)).await?
    };

    let state_path = working_dir.join(format!(".planning-agent/{}.json", feature_name));

    let state = if cli.continue_workflow {
        let _ = output_tx.send(Event::Output(format!("[planning] Loading existing workflow: {}", feature_name)));
        State::load(&state_path)?
    } else {
        let _ = output_tx.send(Event::Output(format!("[planning] Starting new workflow: {}", feature_name)));
        let _ = output_tx.send(Event::Output(format!("[planning] Objective: {}", objective)));
        State::new(&feature_name, &objective, cli.max_iterations)
    };

    app.workflow_state = Some(state.clone());

    // Create docs/plans directory
    let plans_dir = working_dir.join("docs/plans");
    std::fs::create_dir_all(&plans_dir).context("Failed to create docs/plans directory")?;

    // Save initial state
    state.save(&state_path)?;

    // Spawn workflow task
    let workflow_tx = output_tx.clone();
    let workflow_working_dir = working_dir.clone();
    let workflow_state_path = state_path.clone();
    let initial_state = state.clone();

    let workflow_handle = tokio::spawn(async move {
        run_workflow(initial_state, workflow_working_dir, workflow_state_path, workflow_tx).await
    });

    // Main event loop
    loop {
        // Draw UI
        terminal.draw(|frame| tui::ui::draw(frame, &app))?;

        // Handle events
        match event_handler.next().await? {
            Event::Key(key) => {
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
            Event::Quit => {
                app.should_quit = true;
            }
        }

        if app.should_quit {
            break;
        }

        // Check if workflow completed
        if workflow_handle.is_finished() {
            app.running = false;
            // Wait a bit for user to see final state
            tokio::time::sleep(Duration::from_secs(2)).await;
            break;
        }
    }

    // Restore terminal
    restore_terminal(&mut terminal)?;

    // Wait for workflow to complete
    if !workflow_handle.is_finished() {
        workflow_handle.abort();
    }

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

async fn run_workflow(
    mut state: State,
    working_dir: PathBuf,
    state_path: PathBuf,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output("=== PLANNING PHASE ===".to_string()));
                let _ = output_tx.send(Event::Output(format!("Feature: {}", state.feature_name)));
                let _ = output_tx.send(Event::Output(format!("Plan file: {}", state.plan_file.display())));

                run_planning_phase(&state, &working_dir).await?;

                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    let _ = output_tx.send(Event::Output("[error] Plan file was not created!".to_string()));
                    anyhow::bail!("Plan file not created");
                }

                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                let _ = output_tx.send(Event::Output("[planning] Transitioning to review phase...".to_string()));
            }

            Phase::Reviewing => {
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output(format!("=== REVIEW PHASE (Iteration {}) ===", state.iteration)));

                run_review_phase(&state, &working_dir).await?;

                let feedback_path = working_dir.join(&state.feedback_file);
                if !feedback_path.exists() {
                    let _ = output_tx.send(Event::Output("[error] Feedback file was not created!".to_string()));
                    anyhow::bail!("Feedback file not created");
                }

                let status = parse_feedback_status(&feedback_path)?;
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        let _ = output_tx.send(Event::Output("[planning] Plan APPROVED!".to_string()));
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        let _ = output_tx.send(Event::Output("[planning] Plan needs revision".to_string()));
                        if state.iteration >= state.max_iterations {
                            let _ = output_tx.send(Event::Output("[planning] Max iterations reached".to_string()));
                            break;
                        }
                        state.transition(Phase::Revising)?;
                    }
                }
                state.save(&state_path)?;
            }

            Phase::Revising => {
                let _ = output_tx.send(Event::Output("".to_string()));
                let _ = output_tx.send(Event::Output(format!("=== REVISION PHASE (Iteration {}) ===", state.iteration)));

                run_revision_phase(&state, &working_dir).await?;

                state.iteration += 1;
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
                let _ = output_tx.send(Event::Output("[planning] Transitioning to review phase...".to_string()));
            }

            Phase::Complete => break,
        }
    }

    let _ = output_tx.send(Event::Output("".to_string()));
    let _ = output_tx.send(Event::Output("=== WORKFLOW COMPLETE ===".to_string()));
    if state.phase == Phase::Complete {
        let _ = output_tx.send(Event::Output(format!("Plan APPROVED after {} iteration(s)", state.iteration)));
    } else {
        let _ = output_tx.send(Event::Output("Max iterations reached. Manual review recommended.".to_string()));
    }

    Ok(())
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

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                eprintln!("\n=== PLANNING PHASE ===");
                run_planning_phase(&state, &working_dir).await?;

                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    anyhow::bail!("Plan file not created: {}", plan_path.display());
                }

                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
            }

            Phase::Reviewing => {
                eprintln!("\n=== REVIEW PHASE (Iteration {}) ===", state.iteration);
                run_review_phase(&state, &working_dir).await?;

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
                run_revision_phase(&state, &working_dir).await?;

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
