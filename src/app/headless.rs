use crate::app::cli::Cli;
use crate::app::failure::{FailureContext, FailureKind, OnAllReviewersFailed};
use crate::app::util::truncate_for_summary;
use crate::app::workflow_common::{plan_file_has_content, pre_create_plan_files_with_working_dir};
use crate::config::{AgentRef, WorkflowConfig};
use crate::phases::{
    self, aggregate_reviews, merge_feedback, run_multi_agent_review_with_context,
    run_planning_phase_with_context, run_revision_phase_with_context, write_feedback_files,
};
use crate::planning_paths;
use crate::prompt_format::PromptBuilder;
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

    let prompt = PromptBuilder::new()
        .phase("feature-name-extraction")
        .instructions(r#"Extract a short kebab-case feature name (2-4 words, lowercase, hyphens) from the given objective.
Output ONLY the feature name, nothing else.

Example outputs: "sharing-permissions", "user-auth", "api-rate-limiting""#)
        .input("objective", objective)
        .build();

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
                                let max_retries = config.failure_policy.max_retries as usize;
                                if planning_attempts > max_retries {
                                    anyhow::bail!(
                                        "Plan file empty after {} attempts (headless mode does not support interactive recovery)",
                                        planning_attempts
                                    );
                                }
                                sender.send_output(format!(
                                    "[planning] Retrying plan generation ({}/{})...",
                                    planning_attempts, max_retries
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
                            let max_retries = config.failure_policy.max_retries as usize;
                            if planning_attempts > max_retries {
                                anyhow::bail!(
                                    "Planning failed after {} attempts: {} (headless mode does not support interactive recovery)",
                                    planning_attempts,
                                    error_msg
                                );
                            }
                            sender.send_output(format!(
                                "[planning] Retrying plan generation ({}/{})...",
                                planning_attempts, max_retries
                            ));
                            continue;
                        }
                    }
                }

                state.transition(Phase::Reviewing)?;
                state.set_updated_at();
                state.save_atomic(&state_path)?;
            }

            Phase::Reviewing => {
                sender.send_output(format!(
                    "\n=== REVIEW PHASE (Iteration {}) ===",
                    state.iteration
                ));

                let mut reviews_by_agent: HashMap<String, phases::ReviewResult> = HashMap::new();
                let mut pending_reviewers: Vec<AgentRef> =
                    config.workflow.reviewing.agents.clone();
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

                    // Collect failed display_ids
                    let failed_display_ids: Vec<String> = batch
                        .failures
                        .iter()
                        .map(|f| f.agent_name.clone())
                        .collect();

                    // Find the AgentRef objects that match failed display_ids
                    let failed_agent_refs: Vec<AgentRef> = config
                        .workflow
                        .reviewing
                        .agents
                        .iter()
                        .filter(|r| failed_display_ids.contains(&r.display_id().to_string()))
                        .cloned()
                        .collect();

                    let mut has_bundles = false;
                    for failure in &batch.failures {
                        sender.send_output(format!(
                            "[review:{}] failed: {}",
                            failure.agent_name,
                            truncate_for_summary(&failure.error, 160)
                        ));
                        if let Some(ref path) = failure.bundle_path {
                            sender.send_output(format!(
                                "[diagnostics:{}] Bundle: {}",
                                failure.agent_name,
                                path.display()
                            ));
                            has_bundles = true;
                        }
                    }
                    if has_bundles {
                        sender.send_output("[warning] Diagnostics bundles may contain sensitive information from logs.".to_string());
                    }

                    if reviews_by_agent.is_empty() {
                        let max_retries = config.failure_policy.max_retries as usize;
                        if retry_attempts < max_retries {
                            retry_attempts += 1;
                            sender.send_output(format!(
                                "[review] All reviewers failed; retrying ({}/{})...",
                                retry_attempts, max_retries
                            ));
                            pending_reviewers = failed_agent_refs.clone();
                            continue;
                        }
                        // Output bundle paths before applying policy
                        for failure in &batch.failures {
                            if let Some(ref path) = failure.bundle_path {
                                sender.send_output(format!(
                                    "[diagnostics] {}: {}",
                                    failure.agent_name,
                                    path.display()
                                ));
                            }
                        }

                        // Apply failure policy
                        match config.failure_policy.on_all_reviewers_failed {
                            OnAllReviewersFailed::SaveState => {
                                // Set failure context for later recovery
                                state.set_failure(FailureContext {
                                    kind: FailureKind::AllReviewersFailed,
                                    phase: state.phase.clone(),
                                    agent_name: None,
                                    retry_count: retry_attempts as u32,
                                    max_retries: max_retries as u32,
                                    failed_at: chrono::Utc::now().to_rfc3339(),
                                    recovery_action: None,
                                });
                                state.set_updated_at();
                                state.save_atomic(&state_path)?;

                                sender.send_output("[error] All reviewers failed after retries.".to_string());
                                sender.send_output(format!("[save] State saved to: {}", state_path.display()));
                                sender.send_output(format!(
                                    "[info] To recover in TUI mode, run: planning-agent --continue --name {}",
                                    state.feature_name
                                ));
                                return Ok(());
                            }
                            OnAllReviewersFailed::ContinueWithoutReview => {
                                sender.send_output("[warning] All reviewers failed - continuing without review".to_string());
                                // Transition directly to revision phase without feedback
                                if state.iteration >= state.max_iterations {
                                    sender.send_output("[planning] Max iterations reached".to_string());
                                    break;
                                }
                                state.transition(Phase::Revising)?;
                                state.set_updated_at();
                                state.save_atomic(&state_path)?;
                                break; // Exit the review loop
                            }
                            OnAllReviewersFailed::Abort => {
                                // Set failure context for diagnostics before bailing
                                state.set_failure(FailureContext {
                                    kind: FailureKind::AllReviewersFailed,
                                    phase: state.phase.clone(),
                                    agent_name: None,
                                    retry_count: retry_attempts as u32,
                                    max_retries: max_retries as u32,
                                    failed_at: chrono::Utc::now().to_rfc3339(),
                                    recovery_action: None,
                                });
                                state.set_updated_at();
                                let _ = state.save_atomic(&state_path);
                                anyhow::bail!("All reviewers failed to complete review");
                            }
                        }
                    }

                    sender.send_output(format!(
                        "[review] Some reviewers failed: {}. Continuing with {} successful review(s).",
                        failed_display_ids.join(", "),
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
                state.set_updated_at();
                state.save_atomic(&state_path)?;
            }

            Phase::Revising => {
                sender.send_output(format!(
                    "\n=== REVISION PHASE (Iteration {}) ===",
                    state.iteration
                ));
                let iteration = state.iteration;
                let mut revising_attempts = 0usize;
                let max_retries = config.failure_policy.max_retries as usize;

                loop {
                    let revision_result = run_revision_phase_with_context(
                        &mut state,
                        &working_dir,
                        &config,
                        &last_reviews,
                        sender.clone(),
                        iteration,
                        &state_path,
                    )
                    .await;

                    match revision_result {
                        Ok(()) => break,
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            sender.send_output(format!("[revision] Failed: {}", error_msg));

                            revising_attempts += 1;
                            if revising_attempts > max_retries {
                                // Apply failure policy for revising failures
                                match config.failure_policy.on_all_reviewers_failed {
                                    OnAllReviewersFailed::SaveState => {
                                        state.set_failure(FailureContext {
                                            kind: FailureKind::Unknown(error_msg),
                                            phase: state.phase.clone(),
                                            agent_name: Some(config.workflow.revising.agent.clone()),
                                            retry_count: revising_attempts as u32,
                                            max_retries: max_retries as u32,
                                            failed_at: chrono::Utc::now().to_rfc3339(),
                                            recovery_action: None,
                                        });
                                        state.set_updated_at();
                                        state.save_atomic(&state_path)?;

                                        sender.send_output("[error] Revision failed after retries.".to_string());
                                        sender.send_output(format!("[save] State saved to: {}", state_path.display()));
                                        sender.send_output(format!(
                                            "[info] To recover in TUI mode, run: planning-agent --continue --name {}",
                                            state.feature_name
                                        ));
                                        return Ok(());
                                    }
                                    OnAllReviewersFailed::ContinueWithoutReview => {
                                        sender.send_output("[warning] Revision failed - skipping to review phase".to_string());
                                        break;
                                    }
                                    OnAllReviewersFailed::Abort => {
                                        anyhow::bail!(
                                            "Revision failed after {} attempts: {} (headless mode does not support interactive recovery)",
                                            revising_attempts,
                                            error_msg
                                        );
                                    }
                                }
                            }
                            sender.send_output(format!(
                                "[revision] Retrying ({}/{})...",
                                revising_attempts, max_retries
                            ));
                            continue;
                        }
                    }
                }

                last_reviews.clear();

                // Keep old feedback files - don't cleanup
                // let feedback_path = working_dir.join(&state.feedback_file);
                // let _ = cleanup_merged_feedback(&feedback_path);

                state.iteration += 1;
                // Update feedback filename for the new iteration before transitioning to review
                state.update_feedback_for_iteration(state.iteration);
                state.transition(Phase::Reviewing)?;
                state.set_updated_at();
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

    // Output merge instructions if using a worktree
    if let Some(ref wt_state) = state.worktree_info {
        let info = crate::git_worktree::WorktreeInfo {
            worktree_path: wt_state.worktree_path.clone(),
            branch_name: wt_state.branch_name.clone(),
            source_branch: wt_state.source_branch.clone(),
            original_dir: wt_state.original_dir.clone(),
            has_submodules: false,
        };
        let instructions = crate::git_worktree::generate_merge_instructions(&info);
        for line in instructions.lines() {
            sender.send_output(line.to_string());
        }
    }

    Ok(())
}

pub async fn run_headless(cli: Cli) -> Result<()> {
    let working_dir = cli
        .working_dir
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    // --claude flag takes priority over any config file
    let workflow_config = if cli.claude {
        eprintln!("[planning-agent] Using Claude-only workflow config (--claude)");
        WorkflowConfig::claude_only_config()
    } else if let Some(config_path) = &cli.config {
        let full_path = if config_path.is_absolute() {
            config_path.clone()
        } else {
            working_dir.join(config_path)
        };
        match WorkflowConfig::load(&full_path) {
            Ok(cfg) => {
                eprintln!("[planning-agent] Loaded config from {:?}", full_path);
                cfg
            }
            Err(e) => {
                eprintln!("[planning-agent] Warning: Failed to load config: {}", e);
                eprintln!("[planning-agent] Using built-in multi-agent workflow config");
                WorkflowConfig::default_config()
            }
        }
    } else {
        let default_config_path = working_dir.join("workflow.yaml");
        if default_config_path.exists() {
            match WorkflowConfig::load(&default_config_path) {
                Ok(cfg) => {
                    eprintln!("[planning-agent] Loaded default workflow.yaml");
                    cfg
                }
                Err(e) => {
                    eprintln!(
                        "[planning-agent] Warning: Failed to load workflow.yaml: {}",
                        e
                    );
                    eprintln!("[planning-agent] Using built-in multi-agent workflow config");
                    WorkflowConfig::default_config()
                }
            }
        } else {
            eprintln!("[planning-agent] Using built-in multi-agent workflow config");
            WorkflowConfig::default_config()
        }
    };

    let objective = cli.objective.join(" ");

    let feature_name = if let Some(name) = cli.name {
        name
    } else if cli.continue_workflow {
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    } else {
        eprintln!("[planning] Extracting feature name...");
        extract_feature_name(&objective, None).await?
    };

    let state_path = planning_paths::state_path(&working_dir, &feature_name)?;

    let mut state = if cli.continue_workflow {
        eprintln!("[planning] Loading existing workflow: {}", feature_name);
        State::load(&state_path)?
    } else {
        eprintln!("[planning] Starting new workflow: {}", feature_name);
        eprintln!("[planning] Objective: {}", objective);
        State::new(&feature_name, &objective, cli.max_iterations)?
    };

    // Canonicalize working_dir for absolute paths in prompts
    let working_dir = std::fs::canonicalize(&working_dir).unwrap_or(working_dir);

    // Set up git worktree if in a git repository (and not disabled)
    let effective_working_dir = if let Some(ref existing_wt) = state.worktree_info {
        // Worktree already exists from previous session (--continue case)
        if cli.no_worktree {
            eprintln!("[planning] Note: Using existing worktree (--no-worktree only affects new sessions)");
        }
        // Validate it still exists and is a valid git worktree
        if crate::git_worktree::is_valid_worktree(&existing_wt.worktree_path) {
            eprintln!("[planning] Reusing existing worktree: {}", existing_wt.worktree_path.display());
            eprintln!("[planning] Branch: {}", existing_wt.branch_name);
            existing_wt.worktree_path.clone()
        } else {
            eprintln!("[planning] Warning: Previous worktree no longer valid, falling back to original dir");
            let original = existing_wt.original_dir.clone();
            state.worktree_info = None;
            original
        }
    } else if cli.no_worktree {
        eprintln!("[planning] Worktree disabled via --no-worktree");
        working_dir.clone()
    } else {
        // No existing worktree, try to create one
        let session_dir = match crate::planning_paths::session_dir(&state.workflow_session_id) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("[planning] Warning: Could not get session directory: {}", e);
                eprintln!("[planning] Continuing with original directory");
                working_dir.clone()
            }
        };

        // Use custom worktree dir if provided, otherwise use session_dir
        let worktree_base = cli.worktree_dir
            .as_ref()
            .cloned()
            .unwrap_or(session_dir);

        match crate::git_worktree::create_session_worktree(
            &working_dir,
            &state.workflow_session_id,
            &feature_name,
            &worktree_base,
            cli.worktree_branch.as_deref(),
        ) {
            crate::git_worktree::WorktreeSetupResult::Created(info) => {
                eprintln!("[planning] Created git worktree at: {}", info.worktree_path.display());
                eprintln!("[planning] Working on branch: {}", info.branch_name);
                if let Some(ref source) = info.source_branch {
                    eprintln!("[planning] Will merge into: {}", source);
                }
                if info.has_submodules {
                    eprintln!("[planning] Warning: Repository has submodules");
                    eprintln!("[planning] Run 'git submodule update --init' in the worktree if needed.");
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
                eprintln!("[planning] Not a git repository, using original directory");
                working_dir.clone()
            }
            crate::git_worktree::WorktreeSetupResult::Failed(err) => {
                eprintln!("[planning] Warning: Git worktree setup failed: {}", err);
                eprintln!("[planning] Continuing with original directory");
                working_dir.clone()
            }
        }
    };

    // Pre-create plan folder and files (in ~/.planning-agent/sessions/)
    pre_create_plan_files_with_working_dir(&state, Some(&effective_working_dir))
        .context("Failed to pre-create plan files")?;

    state.set_updated_at();
    state.save_atomic(&state_path)?;

    let (output_tx, mut output_rx) = mpsc::unbounded_channel::<Event>();

    tokio::spawn(async move {
        while let Some(event) = output_rx.recv().await {
            match event {
                Event::Output(line) | Event::Streaming(line) => {
                    eprintln!("{}", line);
                }
                Event::ToolStarted {
                    tool_id: _,
                    display_name,
                    input_preview,
                    agent_name,
                } => {
                    if input_preview.is_empty() {
                        eprintln!("[tool started] [{}] {}", agent_name, display_name);
                    } else {
                        eprintln!("[tool started] [{}] {}: {}", agent_name, display_name, input_preview);
                    }
                }
                Event::ToolFinished { tool_id: _, agent_name } => {
                    eprintln!("[tool finished] [{}]", agent_name);
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
                        if let Some(id) = tool_id {
                            eprintln!("[tool error] [{}] {}", agent_name, id);
                        } else {
                            eprintln!("[tool error] [{}]", agent_name);
                        }
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
        effective_working_dir,
        state_path,
        workflow_config,
        output_tx.clone(),
    )
    .await?;

    Ok(())
}
