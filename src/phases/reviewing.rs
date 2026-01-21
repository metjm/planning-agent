use crate::agents::{AgentContext, AgentType};
use crate::app::failure::FailureKind;
use crate::app::workflow_common::is_network_error;
use crate::config::{AgentRef, AggregationMode, WorkflowConfig};
use crate::diagnostics::{create_review_bundle, AttemptTimestamp, BundleConfig};
use crate::phases::review_parser::parse_review_feedback;
use crate::phases::review_prompts::{
    build_review_prompt_for_agent, build_review_recovery_prompt_for_agent, REVIEW_SYSTEM_PROMPT,
};
use crate::phases::review_schema::SubmittedReview;
use crate::phases::reviewing_conversation_key;
use crate::planning_paths;
use crate::session_logger::SessionLogger;
use crate::state::{FeedbackStatus, ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Re-export VerdictParseResult and parse_verdict for external use (used in tests)
#[allow(unused_imports)]
pub use crate::phases::review_parser::{parse_verdict, VerdictParseResult};

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
    /// Short summary of the review (from structured review or extracted from feedback)
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct ReviewFailure {
    pub agent_name: String,
    pub error: String,
    /// Path to the diagnostics bundle (if created after retry failure)
    pub bundle_path: Option<PathBuf>,
    /// Classified failure type for recovery decisions
    #[allow(dead_code)]
    pub kind: FailureKind,
}

impl ReviewFailure {
    /// Returns true if this failure is potentially recoverable via retry.
    #[allow(dead_code)]
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }
}

#[derive(Debug, Clone)]
pub struct ReviewBatchResult {
    pub reviews: Vec<ReviewResult>,
    pub failures: Vec<ReviewFailure>,
}

/// Result from executing a single review attempt
struct ReviewAttemptResult {
    output: String,
    #[allow(dead_code)]
    is_error: bool,
    started_at: String,
    ended_at: String,
}

/// Result from the full review execution (possibly including retry)
enum ReviewExecutionResult {
    /// Successfully parsed review
    Success(SubmittedReview),
    /// Failed to parse even after retry - includes both outputs and metadata for bundle
    ParseFailure {
        error: String,
        plan_feedback_found: bool,
        verdict_found: bool,
        initial_output: String,
        retry_output: Option<String>,
        attempt_timestamps: Vec<AttemptTimestamp>,
        feedback_file_path: PathBuf,
    },
    /// Agent execution error (not a parse failure)
    ExecutionError(String),
}

/// Generate a stable feedback file path for a review agent.
/// The path is deterministic based on session_id and agent_name, so recovery attempts
/// use the same file.
fn generate_feedback_file_path(
    session_id: &str,
    agent_name: &str,
    iteration: u32,
) -> Result<PathBuf> {
    // Use session directory for feedback files
    let session_dir = planning_paths::session_dir(session_id)?;
    let filename = format!("feedback_{}_{}.md", iteration, agent_name);
    Ok(session_dir.join(filename))
}

#[allow(clippy::too_many_arguments)]
pub async fn run_multi_agent_review_with_context(
    state: &mut State,
    working_dir: &Path,
    config: &WorkflowConfig,
    agent_refs: &[AgentRef],
    session_sender: SessionEventSender,
    iteration: u32,
    state_path: &Path,
    session_logger: Arc<SessionLogger>,
    emit_round_started: bool,
) -> Result<ReviewBatchResult> {
    if agent_refs.is_empty() {
        anyhow::bail!("No reviewers configured");
    }

    let display_ids: Vec<&str> = agent_refs.iter().map(|r| r.display_id()).collect();

    // Conditionally output message based on whether this is sequential or parallel
    if agent_refs.len() == 1 {
        session_sender.send_output(format!(
            "[review] Running reviewer: {}",
            display_ids[0]
        ));
    } else {
        session_sender.send_output(format!(
            "[review] Running {} reviewer(s) in parallel: {}",
            agent_refs.len(),
            display_ids.join(", ")
        ));
    }

    // Only emit round started if caller requests it
    if emit_round_started {
        session_sender.send_review_round_started(iteration);
    }

    let phase_name = format!("Reviewing #{}", iteration);

    // Check if tags are required from config
    let require_tags = config.workflow.reviewing.require_plan_feedback_tags;

    // Build agent contexts: (display_id, conversation_id, resume_strategy, custom_prompt)
    let mut agent_contexts: Vec<(String, Option<String>, ResumeStrategy, Option<String>)> =
        Vec::new();
    for agent_ref in agent_refs {
        let display_id = agent_ref.display_id().to_string();
        let custom_prompt = agent_ref.custom_prompt().map(|s| s.to_string());

        // Reviewing always uses Stateless - each review should be independent
        // with a fresh perspective on the plan, not influenced by prior sessions.
        let resume_strategy = ResumeStrategy::Stateless;
        // Use namespaced session key to avoid collisions with planning sessions
        let conversation_id_name = reviewing_conversation_key(&display_id);
        let agent_session = state.get_or_create_agent_session(&conversation_id_name, resume_strategy.clone());
        agent_contexts.push((
            display_id.clone(),
            agent_session.conversation_id.clone(),
            agent_session.resume_strategy.clone(),
            custom_prompt,
        ));
        state.record_invocation(&conversation_id_name, &phase_name);
    }
    state.set_updated_at();
    state.save_atomic(state_path)?;

    // Build agents: (display_id, AgentType, conversation_id, resume_strategy, custom_prompt)
    #[allow(clippy::type_complexity)]
    let agents: Vec<(String, AgentType, Option<String>, ResumeStrategy, Option<String>)> =
        agent_refs
            .iter()
            .zip(agent_contexts.into_iter())
            .map(
                |(agent_ref, (display_id, conversation_id, resume_strategy, custom_prompt))| {
                    let agent_name = agent_ref.agent_name();
                    let agent_config = config.get_agent(agent_name).ok_or_else(|| {
                        anyhow::anyhow!("Review agent '{}' not found in config", agent_name)
                    })?;
                    Ok((
                        display_id,
                        AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?,
                        conversation_id,
                        resume_strategy,
                        custom_prompt,
                    ))
                },
            )
            .collect::<Result<Vec<_>>>()?;

    // Get plan path (absolute)
    let plan_path = state.plan_file.clone();
    let plan_path_abs = if plan_path.is_absolute() {
        plan_path.clone()
    } else {
        working_dir.join(&plan_path)
    };

    // Get objective for prompts
    let objective = state.objective.clone();
    let session_id = state.workflow_session_id.clone();

    let futures: Vec<_> = agents
        .into_iter()
        .map(|(display_id, agent, conversation_id, resume_strategy, custom_prompt)| {
            let sender = session_sender.clone();
            let phase = format!("Reviewing #{}", iteration);
            let logger = session_logger.clone();
            let working_dir = working_dir.to_path_buf();
            let plan_path_abs = plan_path_abs.clone();
            let objective = objective.clone();
            let session_id = session_id.clone();
            let iter = iteration;

            // System prompt is minimal - skill handles details
            let system_prompt = REVIEW_SYSTEM_PROMPT.to_string();

            async move {
                // Record start time for duration computation
                let review_started_at = std::time::Instant::now();

                // Signal reviewer started
                sender.send_reviewer_started(iter, display_id.clone());

                sender.send_output(format!("[review:{}] Starting file-based review...", display_id));

                // Generate stable feedback file path for this agent
                let feedback_path = match generate_feedback_file_path(&session_id, &display_id, iter) {
                    Ok(p) => p,
                    Err(e) => {
                        let duration_ms = review_started_at.elapsed().as_millis() as u64;
                        return (
                            display_id,
                            ReviewExecutionResult::ExecutionError(format!(
                                "Failed to generate feedback path: {}",
                                e
                            )),
                            duration_ms,
                        );
                    }
                };

                // Compute session folder for supplementary file access
                let session_folder = match planning_paths::session_dir(&session_id) {
                    Ok(p) => p,
                    Err(e) => {
                        let duration_ms = review_started_at.elapsed().as_millis() as u64;
                        return (
                            display_id,
                            ReviewExecutionResult::ExecutionError(format!(
                                "Failed to get session folder: {}",
                                e
                            )),
                            duration_ms,
                        );
                    }
                };

                // Build the review prompt with custom focus if present
                let review_prompt = build_review_prompt_for_agent(
                    &objective,
                    &plan_path_abs,
                    &feedback_path,
                    &working_dir,
                    &session_folder,
                    require_tags,
                    custom_prompt.as_deref(),
                );

                sender.send_output(format!(
                    "[review:{}] Plan: {}, Feedback: {}",
                    display_id,
                    plan_path_abs.display(),
                    feedback_path.display()
                ));

                // First attempt
                let attempt1_result = execute_review_attempt(
                    &agent,
                    &review_prompt,
                    &conversation_id,
                    &resume_strategy,
                    &sender,
                    &phase,
                    &system_prompt,
                    logger.clone(),
                )
                .await;

                let (initial_output, attempt1_timestamp) = match attempt1_result {
                    Ok(result) => (result.output, AttemptTimestamp {
                        attempt: 1,
                        started_at: result.started_at,
                        ended_at: result.ended_at,
                    }),
                    Err(e) => {
                        let duration_ms = review_started_at.elapsed().as_millis() as u64;
                        return (
                            display_id,
                            ReviewExecutionResult::ExecutionError(e.to_string()),
                            duration_ms,
                        );
                    }
                };

                // Try to read and parse the feedback file
                match try_parse_feedback_file(&feedback_path, require_tags) {
                    Ok(review) => {
                        sender.send_output(format!("[review:{}] Review complete", display_id));
                        let duration_ms = review_started_at.elapsed().as_millis() as u64;
                        (display_id, ReviewExecutionResult::Success(review), duration_ms)
                    }
                    Err(parse_failure) => {
                        // Initial attempt failed - try recovery
                        sender.send_output(format!(
                            "[review:{}] Failed to parse feedback: {}. Attempting recovery...",
                            display_id, parse_failure.error
                        ));

                        // Build recovery prompt
                        let recovery_prompt = build_review_recovery_prompt_for_agent(
                            &plan_path_abs,
                            &feedback_path,
                            &session_folder,
                            &parse_failure.error,
                            &initial_output,
                            require_tags,
                        );

                        // Execute retry attempt
                        let attempt2_result = execute_review_attempt(
                            &agent,
                            &recovery_prompt,
                            &conversation_id,
                            &resume_strategy,
                            &sender,
                            &format!("{} (recovery)", phase),
                            &system_prompt,
                            logger.clone(),
                        )
                        .await;

                        let (retry_output, attempt2_timestamp) = match attempt2_result {
                            Ok(result) => (
                                Some(result.output.clone()),
                                Some(AttemptTimestamp {
                                    attempt: 2,
                                    started_at: result.started_at,
                                    ended_at: result.ended_at,
                                }),
                            ),
                            Err(e) => {
                                sender.send_output(format!(
                                    "[review:{}] Recovery attempt failed: {}",
                                    display_id, e
                                ));
                                (None, None)
                            }
                        };

                        // Try to parse feedback file after recovery
                        match try_parse_feedback_file(&feedback_path, require_tags) {
                            Ok(review) => {
                                sender.send_output(format!(
                                    "[review:{}] Recovery succeeded!",
                                    display_id
                                ));
                                let duration_ms = review_started_at.elapsed().as_millis() as u64;
                                (display_id, ReviewExecutionResult::Success(review), duration_ms)
                            }
                            Err(final_failure) => {
                                // Both attempts failed - prepare for bundle creation
                                let mut timestamps = vec![attempt1_timestamp];
                                if let Some(ts) = attempt2_timestamp {
                                    timestamps.push(ts);
                                }

                                sender.send_output(format!(
                                    "[review:{}] Recovery failed. Diagnostics bundle will be created.",
                                    display_id
                                ));

                                let duration_ms = review_started_at.elapsed().as_millis() as u64;
                                (
                                    display_id,
                                    ReviewExecutionResult::ParseFailure {
                                        error: final_failure.error,
                                        plan_feedback_found: final_failure.plan_feedback_found,
                                        verdict_found: final_failure.verdict_found,
                                        initial_output,
                                        retry_output,
                                        attempt_timestamps: timestamps,
                                        feedback_file_path: feedback_path,
                                    },
                                    duration_ms,
                                )
                            }
                        }
                    }
                }
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // Get run_id for bundle creation
    let run_id = crate::app::util::get_run_id();

    let mut reviews = Vec::new();
    let mut failures = Vec::new();

    for (agent_name, result, duration_ms) in results {
        match result {
            ReviewExecutionResult::Success(review) => {
                let needs_revision = review.needs_revision();
                let feedback = review.feedback_content();

                let verdict_str = if needs_revision {
                    "NEEDS REVISION"
                } else {
                    "APPROVED"
                };

                session_sender.send_output(format!(
                    "[review:{}] Verdict: {}",
                    agent_name, verdict_str
                ));

                // Use summary, with fallback to default if empty
                let summary = if review.summary.trim().is_empty() {
                    "Review completed".to_string()
                } else {
                    review.summary.clone()
                };

                // Send reviewer completed event with duration
                session_sender.send_reviewer_completed(
                    iteration,
                    agent_name.clone(),
                    !needs_revision, // approved = !needs_revision
                    summary.clone(),
                    duration_ms,
                );

                reviews.push(ReviewResult {
                    agent_name,
                    needs_revision,
                    feedback,
                    summary,
                });
            }
            ReviewExecutionResult::ParseFailure {
                error,
                plan_feedback_found,
                verdict_found,
                initial_output,
                retry_output,
                attempt_timestamps,
                feedback_file_path,
            } => {
                // Create diagnostics bundle
                let bundle_config = BundleConfig {
                    working_dir,
                    agent_name: &agent_name,
                    failure_reason: &error,
                    server_name: &format!("file-review-{}", agent_name),
                    run_id: &run_id,
                    plan_feedback_found,
                    verdict_found,
                    attempt_timestamps,
                    initial_output: Some(&initial_output),
                    retry_output: retry_output.as_deref(),
                    state_path: Some(state_path),
                    plan_file: Some(&state.plan_file),
                    feedback_file: Some(&feedback_file_path),
                    workflow_session_id: Some(&state.workflow_session_id),
                };

                let bundle_path = create_review_bundle(bundle_config);

                if let Some(ref path) = bundle_path {
                    session_sender.send_output(format!(
                        "[review:{}] Diagnostics bundle created: {}",
                        agent_name,
                        path.display()
                    ));
                }

                let full_error = format!(
                    "Failed to parse review verdict after retry. {}",
                    error
                );
                session_sender.send_output(format!("[review:{}] ERROR: {}", agent_name, full_error));

                // Send reviewer failed event
                session_sender.send_reviewer_failed(
                    iteration,
                    agent_name.clone(),
                    full_error.clone(),
                );

                failures.push(ReviewFailure {
                    agent_name,
                    error: full_error.clone(),
                    bundle_path,
                    kind: FailureKind::ParseFailure(full_error),
                });
            }
            ReviewExecutionResult::ExecutionError(error) => {
                session_sender.send_output(format!("[error] {} review failed: {}", agent_name, error));

                // Send reviewer failed event
                session_sender.send_reviewer_failed(
                    iteration,
                    agent_name.clone(),
                    error.clone(),
                );

                // Classify the error based on its content
                let kind = classify_execution_error(&error);
                failures.push(ReviewFailure {
                    agent_name,
                    error,
                    bundle_path: None,
                    kind,
                });
            }
        }
    }

    Ok(ReviewBatchResult { reviews, failures })
}

/// Try to read and parse the feedback file
fn try_parse_feedback_file(
    feedback_path: &Path,
    require_tags: bool,
) -> Result<SubmittedReview, crate::diagnostics::ParseFailureInfo> {
    // Check if the file exists
    if !feedback_path.exists() {
        return Err(crate::diagnostics::ParseFailureInfo {
            error: format!("Feedback file not found: {}", feedback_path.display()),
            plan_feedback_found: false,
            verdict_found: false,
        });
    }

    // Read the file content
    let content = match fs::read_to_string(feedback_path) {
        Ok(c) => c,
        Err(e) => {
            return Err(crate::diagnostics::ParseFailureInfo {
                error: format!("Failed to read feedback file: {}", e),
                plan_feedback_found: false,
                verdict_found: false,
            });
        }
    };

    if content.trim().is_empty() {
        return Err(crate::diagnostics::ParseFailureInfo {
            error: "Feedback file is empty".to_string(),
            plan_feedback_found: false,
            verdict_found: false,
        });
    }

    // Parse the content
    parse_review_feedback(&content, require_tags)
}

/// Execute a single review attempt and track timing
#[allow(clippy::too_many_arguments)]
async fn execute_review_attempt(
    agent: &AgentType,
    prompt: &str,
    conversation_id: &Option<String>,
    resume_strategy: &ResumeStrategy,
    sender: &SessionEventSender,
    phase: &str,
    system_prompt: &str,
    session_logger: Arc<SessionLogger>,
) -> Result<ReviewAttemptResult> {
    let started_at = chrono::Utc::now().to_rfc3339();

    let context = AgentContext {
        session_sender: sender.clone(),
        phase: phase.to_string(),
        conversation_id: conversation_id.clone(),
        resume_strategy: resume_strategy.clone(),
        cancel_rx: None,
        session_logger,
    };

    let result = agent
        .execute_streaming_with_context(
            prompt.to_string(),
            Some(system_prompt.to_string()),
            None,
            context,
        )
        .await?;

    let ended_at = chrono::Utc::now().to_rfc3339();

    Ok(ReviewAttemptResult {
        output: result.output,
        is_error: result.is_error,
        started_at,
        ended_at,
    })
}

/// Classifies an execution error into a FailureKind based on error message patterns.
fn classify_execution_error(error: &str) -> FailureKind {
    let error_lower = error.to_lowercase();

    // Check for timeout patterns
    if error_lower.contains("timeout")
        || error_lower.contains("no output for")
        || error_lower.contains("unresponsive")
    {
        return FailureKind::Timeout;
    }

    // Check for network errors
    if is_network_error(error) {
        return FailureKind::Network;
    }

    // Check for empty output patterns
    if error_lower.contains("empty output")
        || error_lower.contains("no content")
        || error_lower.contains("empty response")
    {
        return FailureKind::EmptyOutput;
    }

    // Check for process exit patterns (e.g., "exit code 1")
    if let Some(captures) = regex::Regex::new(r"exit\s*(?:code|status)?\s*[:\s]?\s*(\d+)")
        .ok()
        .and_then(|re| re.captures(&error_lower))
    {
        if let Some(code_str) = captures.get(1) {
            if let Ok(code) = code_str.as_str().parse::<i32>() {
                if code != 0 {
                    return FailureKind::ProcessExit(code);
                }
            }
        }
    }

    // Fallback to unknown
    FailureKind::Unknown(error.chars().take(500).collect())
}

pub fn aggregate_reviews(reviews: &[ReviewResult], mode: &AggregationMode) -> FeedbackStatus {
    if reviews.is_empty() {
        return FeedbackStatus::NeedsRevision;
    }

    let rejections = reviews.iter().filter(|r| r.needs_revision).count();
    let total = reviews.len();

    match mode {
        AggregationMode::AnyRejects => {
            if rejections > 0 {
                FeedbackStatus::NeedsRevision
            } else {
                FeedbackStatus::Approved
            }
        }
        AggregationMode::AllReject => {
            if rejections == total {
                FeedbackStatus::NeedsRevision
            } else {
                FeedbackStatus::Approved
            }
        }
        AggregationMode::Majority => {
            if rejections > total / 2 {
                FeedbackStatus::NeedsRevision
            } else {
                FeedbackStatus::Approved
            }
        }
    }
}

fn feedback_path_for_agent(
    base_feedback_path: &Path,
    agent_name: &str,
    total_reviewers: usize,
) -> PathBuf {
    if total_reviewers <= 1 {
        return base_feedback_path.to_path_buf();
    }

    let stem = base_feedback_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("feedback");
    let ext = base_feedback_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("md");

    base_feedback_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{}_{}.{}", stem, agent_name, ext))
}

/// Write content to a file atomically using temp-file-then-rename pattern.
/// This prevents data corruption if the process crashes during write.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, content)
        .map_err(|e| anyhow::anyhow!("Failed to write temp file {}: {}", temp_path.display(), e))?;
    fs::rename(&temp_path, path)
        .map_err(|e| anyhow::anyhow!("Failed to rename {} to {}: {}", temp_path.display(), path.display(), e))?;
    Ok(())
}

pub fn write_feedback_files(
    reviews: &[ReviewResult],
    base_feedback_path: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    let mut paths = Vec::new();

    for review in reviews {
        let feedback_path =
            feedback_path_for_agent(base_feedback_path, &review.agent_name, reviews.len());
        write_atomic(&feedback_path, &review.feedback)?;
        paths.push(feedback_path);
    }

    Ok(paths)
}

pub fn merge_feedback(reviews: &[ReviewResult], output_path: &Path) -> Result<()> {
    let merged = reviews
        .iter()
        .map(|r| {
            format!(
                "## {} Review\n\n{}\n",
                r.agent_name.to_uppercase(),
                r.feedback
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n\n");

    let header = format!(
        "# Consolidated Review Feedback\n\n{} reviewer(s): {}\n\n---\n\n",
        reviews.len(),
        reviews
            .iter()
            .map(|r| r.agent_name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    write_atomic(output_path, &format!("{}{}", header, merged))
}

#[cfg(test)]
#[path = "reviewing_tests.rs"]
mod reviewing_tests;

// Tests moved to reviewing_tests.rs to keep this file under the line limit
