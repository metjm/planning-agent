mod claude;
mod phases;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use phases::{run_planning_phase, run_review_phase, run_revision_phase};
use state::{parse_feedback_status, FeedbackStatus, Phase, State};
use std::path::PathBuf;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn extract_feature_name(objective: &str) -> Result<String> {
    use std::process::Stdio;
    use tokio::process::Command;

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

async fn run(cli: Cli) -> Result<()> {
    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let objective = cli.objective.join(" ");

    // Get or generate feature name
    let feature_name = if let Some(name) = cli.name {
        name
    } else if cli.continue_workflow {
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    } else {
        eprintln!("[planning] Extracting feature name...");
        extract_feature_name(&objective).await?
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

    // Create docs/plans directory if it doesn't exist
    let plans_dir = working_dir.join("docs/plans");
    std::fs::create_dir_all(&plans_dir)
        .context("Failed to create docs/plans directory")?;

    // Save initial state
    state.save(&state_path)?;

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                eprintln!("\n========================================");
                eprintln!("PLANNING PHASE");
                eprintln!("========================================");
                eprintln!("Feature: {}", state.feature_name);
                eprintln!("Objective: {}", state.objective);
                eprintln!("Plan file: {}", state.plan_file.display());
                eprintln!("----------------------------------------\n");

                run_planning_phase(&state, &working_dir).await?;

                // Verify plan file was created
                let plan_path = working_dir.join(&state.plan_file);
                if !plan_path.exists() {
                    anyhow::bail!("Planning phase completed but plan file was not created: {}", plan_path.display());
                }

                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
            }

            Phase::Reviewing => {
                eprintln!("\n========================================");
                eprintln!("REVIEW PHASE (Iteration {})", state.iteration);
                eprintln!("========================================");
                eprintln!("Reviewing plan: {}", state.plan_file.display());
                eprintln!("Feedback file: {}", state.feedback_file.display());
                eprintln!("----------------------------------------\n");

                run_review_phase(&state, &working_dir).await?;

                // Parse feedback to determine next phase
                let feedback_path = working_dir.join(&state.feedback_file);
                if !feedback_path.exists() {
                    anyhow::bail!("Review phase completed but feedback file was not created: {}", feedback_path.display());
                }

                let status = parse_feedback_status(&feedback_path)?;
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        eprintln!("\n[planning-agent] Plan APPROVED!");
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        eprintln!("\n[planning-agent] Plan needs revision");
                        if state.iteration >= state.max_iterations {
                            eprintln!("[planning-agent] Max iterations reached, stopping");
                            break;
                        }
                        state.transition(Phase::Revising)?;
                    }
                }
                state.save(&state_path)?;
            }

            Phase::Revising => {
                eprintln!("\n========================================");
                eprintln!("REVISION PHASE (Iteration {})", state.iteration);
                eprintln!("========================================");
                eprintln!("Revising plan: {}", state.plan_file.display());
                eprintln!("Based on feedback: {}", state.feedback_file.display());
                eprintln!("----------------------------------------\n");

                run_revision_phase(&state, &working_dir).await?;

                state.iteration += 1;
                state.transition(Phase::Reviewing)?;
                state.save(&state_path)?;
            }

            Phase::Complete => break,
        }
    }

    // Final summary
    eprintln!("\n========================================");
    eprintln!("WORKFLOW COMPLETE");
    eprintln!("========================================");

    if state.phase == Phase::Complete {
        eprintln!("Plan APPROVED after {} iteration(s)", state.iteration);
        eprintln!("Plan file: {}", working_dir.join(&state.plan_file).display());
    } else {
        eprintln!("Max iterations ({}) reached", state.max_iterations);
        eprintln!("Manual review recommended");
        eprintln!("Plan file: {}", working_dir.join(&state.plan_file).display());
        eprintln!("Feedback file: {}", working_dir.join(&state.feedback_file).display());
    }

    Ok(())
}
