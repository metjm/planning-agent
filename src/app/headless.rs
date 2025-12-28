
use crate::app::cli::Cli;
use crate::app::util::truncate_for_summary;
use crate::app::workflow_common::{plan_file_has_content, pre_create_plan_files, REVIEW_FAILURE_RETRY_LIMIT, PLANNING_FAILURE_RETRY_LIMIT};
use crate::config::WorkflowConfig;
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    run_planning_phase_with_context, run_revision_phase_with_context, write_feedback_files,
};
use crate::state::{FeedbackStatus, Phase, State};
use crate::tui::{Event, SessionEventSender};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub async fn extract_feature_name(
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

pub async fn run_headless_with_config(
    mut state: State,
    working_dir: PathBuf,
    state_path: PathBuf,
    config: WorkflowConfig,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let sender = SessionEventSender::new(0, 0, output_tx.clone());
    let mut last_reviews: Vec<phases::ReviewResult> = Vec::new();

    sender.send_output(format!(
        "[session] Workflow session ID: {}",
        state.workflow_session_id
    ));

    while state.should_continue() {
        match state.phase {
            Phase::Planning => {
                sender.send_output("\n=== PLANNING PHASE ===".to_string());

                // state.plan_file is now an absolute path (in ~/.planning-agent/plans/)
                let plan_path = state.plan_file.clone();
                let mut planning_attempts = 0usize;

                loop {
                    let planning_result = run_planning_phase_with_context(
                        &mut state,
                        &working_dir,
                        &config,
                        sender.clone(),
                        &state_path,
                    )
                    .await;

                    match planning_result {
                        Ok(()) => {
                            // Use content-based check instead of exists() for pre-created files
                            if plan_file_has_content(&plan_path) {
                                // Success - plan file has content
                                break;
                            } else {
                                sender.send_output(
                                    "[error] Plan file has no content - planning agent may have failed"
                                        .to_string(),
                                );
                                planning_attempts += 1;
                                if planning_attempts > PLANNING_FAILURE_RETRY_LIMIT {
                                    anyhow::bail!(
                                        "Plan file empty after {} attempts (headless mode does not support interactive recovery)",
                                        planning_attempts
                                    );
                                }
                                sender.send_output(format!(
                                    "[planning] Retrying plan generation ({}/{})...",
                                    planning_attempts, PLANNING_FAILURE_RETRY_LIMIT
                                ));
                                continue;
                            }
                        }
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            sender.send_output(format!("[error] Planning failed: {}", error_msg));

                            // Check if we can continue with an existing plan file that has content
                            if plan_file_has_content(&plan_path) {
                                sender.send_output(
                                    "[planning] Continuing with existing plan file...".to_string(),
                                );
                                break;
                            }

                            planning_attempts += 1;
                            if planning_attempts > PLANNING_FAILURE_RETRY_LIMIT {
                                anyhow::bail!(
                                    "Planning failed after {} attempts: {} (headless mode does not support interactive recovery)",
                                    planning_attempts,
                                    error_msg
                                );
                            }
                            sender.send_output(format!(
                                "[planning] Retrying plan generation ({}/{})...",
                                planning_attempts, PLANNING_FAILURE_RETRY_LIMIT
                            ));
                            continue;
                        }
                    }
                }

                state.transition(Phase::Reviewing)?;
                state.save_atomic(&state_path)?;
            }

            Phase::Reviewing => {
                sender.send_output(format!(
                    "\n=== REVIEW PHASE (Iteration {}) ===",
                    state.iteration
                ));

                let mut reviews_by_agent: HashMap<String, phases::ReviewResult> = HashMap::new();
                let mut pending_reviewers = config.workflow.reviewing.agents.clone();
                let mut retry_attempts = 0usize;

                loop {
                    let iteration = state.iteration;
                    let batch = run_multi_agent_review_with_context(
                        &mut state,
                        &working_dir,
                        &config,
                        &pending_reviewers,
                        sender.clone(),
                        iteration,
                        &state_path,
                    )
                    .await?;

                    for review in batch.reviews {
                        reviews_by_agent.insert(review.agent_name.clone(), review);
                    }

                    if batch.failures.is_empty() {
                        break;
                    }

                    let failed_names = batch
                        .failures
                        .iter()
                        .map(|f| f.agent_name.clone())
                        .collect::<Vec<_>>();

                    for failure in &batch.failures {
                        sender.send_output(format!(
                            "[review:{}] failed: {}",
                            failure.agent_name,
                            truncate_for_summary(&failure.error, 160)
                        ));
                    }

                    if reviews_by_agent.is_empty() {
                        if retry_attempts < REVIEW_FAILURE_RETRY_LIMIT {
                            retry_attempts += 1;
                            sender.send_output(format!(
                                "[review] All reviewers failed; retrying ({}/{})...",
                                retry_attempts, REVIEW_FAILURE_RETRY_LIMIT
                            ));
                            pending_reviewers = failed_names;
                            continue;
                        }
                        anyhow::bail!("All reviewers failed to complete review");
                    }

                    sender.send_output(format!(
                        "[review] Some reviewers failed: {}. Continuing with {} successful review(s).",
                        failed_names.join(", "),
                        reviews_by_agent.len()
                    ));
                    break;
                }

                let mut reviews: Vec<phases::ReviewResult> =
                    reviews_by_agent.into_values().collect();
                reviews.sort_by(|a, b| a.agent_name.cmp(&b.agent_name));

                // state.feedback_file is now an absolute path (in ~/.planning-agent/plans/)
                let feedback_path = state.feedback_file.clone();
                let _ = write_feedback_files(&reviews, &feedback_path);
                let _ = merge_feedback(&reviews, &feedback_path);

                let status = aggregate_reviews(&reviews, &config.workflow.reviewing.aggregation);
                state.last_feedback_status = Some(status.clone());

                match status {
                    FeedbackStatus::Approved => {
                        sender.send_output("[planning] Plan APPROVED!".to_string());
                        state.transition(Phase::Complete)?;
                    }
                    FeedbackStatus::NeedsRevision => {
                        sender.send_output("[planning] Plan needs revision".to_string());
                        if state.iteration >= state.max_iterations {
                            sender.send_output("[planning] Max iterations reached".to_string());
                            break;
                        }
                        state.transition(Phase::Revising)?;
                    }
                }
                last_reviews = reviews;
                state.save_atomic(&state_path)?;
            }

            Phase::Revising => {
                sender.send_output(format!(
                    "\n=== REVISION PHASE (Iteration {}) ===",
                    state.iteration
                ));
                let iteration = state.iteration;
                run_revision_phase_with_context(
                    &mut state,
                    &working_dir,
                    &config,
                    &last_reviews,
                    sender.clone(),
                    iteration,
                    &state_path,
                )
                .await?;
                last_reviews.clear();

                // Keep old feedback files - don't cleanup
                // let feedback_path = working_dir.join(&state.feedback_file);
                // let _ = cleanup_merged_feedback(&feedback_path);

                state.iteration += 1;
                // Update feedback filename for the new iteration before transitioning to review
                state.update_feedback_for_iteration(state.iteration);
                state.transition(Phase::Reviewing)?;
                state.save_atomic(&state_path)?;
            }

            Phase::Complete => break,
        }
    }

    sender.send_output("\n=== WORKFLOW COMPLETE ===".to_string());
    if state.phase == Phase::Complete {
        sender.send_output(format!(
            "Plan APPROVED after {} iteration(s)",
            state.iteration
        ));
        sender.send_output(format!(
            "Plan file: {}",
            state.plan_file.display()
        ));
    } else {
        sender.send_output("Max iterations reached. Manual review recommended.".to_string());
    }

    Ok(())
}

pub async fn run_headless(cli: Cli) -> Result<()> {
    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    let workflow_config = if let Some(config_path) = &cli.config {
        let full_path = if config_path.is_absolute() {
            config_path.clone()
        } else {
            working_dir.join(config_path)
        };
        match WorkflowConfig::load(&full_path) {
            Ok(cfg) => {
                eprintln!("[planning-agent] Loaded config from {:?}", full_path);
                Some(cfg)
            }
            Err(e) => {
                eprintln!("[planning-agent] Warning: Failed to load config: {}", e);
                None
            }
        }
    } else {
        let default_config_path = working_dir.join("workflow.yaml");
        if default_config_path.exists() {
            match WorkflowConfig::load(&default_config_path) {
                Ok(cfg) => {
                    eprintln!("[planning-agent] Loaded default workflow.yaml");
                    Some(cfg)
                }
                Err(e) => {
                    eprintln!(
                        "[planning-agent] Warning: Failed to load workflow.yaml: {}",
                        e
                    );
                    None
                }
            }
        } else {
            None
        }
    };
    let workflow_config = workflow_config.unwrap_or_else(|| {
        eprintln!("[planning-agent] Using built-in multi-agent workflow config");
        WorkflowConfig::default_config()
    });

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

    let state = if cli.continue_workflow {
        eprintln!("[planning] Loading existing workflow: {}", feature_name);
        State::load(&state_path)?
    } else {
        eprintln!("[planning] Starting new workflow: {}", feature_name);
        eprintln!("[planning] Objective: {}", objective);
        State::new(&feature_name, &objective, cli.max_iterations)
    };

    // Canonicalize working_dir for absolute paths in prompts
    let working_dir = std::fs::canonicalize(&working_dir).unwrap_or(working_dir);

    // Pre-create plan folder and files (in ~/.planning-agent/plans/)
    pre_create_plan_files(&state).context("Failed to pre-create plan files")?;

    state.save_atomic(&state_path)?;

    let (output_tx, mut output_rx) = mpsc::unbounded_channel::<Event>();

    tokio::spawn(async move {
        while let Some(event) = output_rx.recv().await {
            match event {
                Event::Output(line) | Event::Streaming(line) => {
                    eprintln!("{}", line);
                }
                Event::ToolStarted { name, agent_name } => {
                    eprintln!("[tool started] [{}] {}", agent_name, name);
                }
                Event::ToolFinished { id, agent_name } => {
                    eprintln!("[tool finished] [{}] {}", agent_name, id);
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
                Event::ToolResultReceived { tool_id, is_error, agent_name } => {
                    if is_error {
                        eprintln!("[tool error] [{}] {}", agent_name, tool_id);
                    }
                }
                Event::StopReason(reason) => {
                    eprintln!("[stop] {}", reason);
                }
                _ => {}
            }
        }
    });

    run_headless_with_config(
        state,
        working_dir,
        state_path,
        workflow_config,
        output_tx.clone(),
    )
    .await?;

    Ok(())
}
