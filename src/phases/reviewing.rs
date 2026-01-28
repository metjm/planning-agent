use crate::agents::{AgentContext, AgentType};
use crate::app::workflow_common::is_network_error;
use crate::app::{create_review_bundle, AttemptTimestamp, BundleConfig};
use crate::config::{AgentRef, AggregationMode, WorkflowConfig};
use crate::domain::actor::WorkflowMessage;
use crate::domain::failure::FailureKind;
use crate::domain::types::{
    AgentId, ConversationId, FeedbackStatus, PhaseLabel, ResumeStrategy as DomainResumeStrategy,
    ResumeStrategy,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowCommand as DomainCommand;
use crate::phases::review_parser::parse_review_feedback;
use crate::phases::review_prompts::{
    build_review_prompt_for_agent, build_review_recovery_prompt_for_agent, DEFAULT_REVIEW_SKILL,
    REVIEW_SYSTEM_PROMPT,
};
use crate::phases::review_schema::SubmittedReview;
use crate::phases::reviewing_conversation_key;
use crate::planning_paths;
use crate::session_daemon::{LogCategory, LogLevel, SessionLogger};
use crate::tui::{ReviewKind, SessionEventSender};
use anyhow::Result;
use ractor::ActorRef;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot;

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
    pub kind: FailureKind,
}

#[derive(Debug, Clone)]
pub struct ReviewBatchResult {
    pub reviews: Vec<ReviewResult>,
    pub failures: Vec<ReviewFailure>,
}

/// Result from executing a single review attempt
struct ReviewAttemptResult {
    output: String,
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
    view: &WorkflowView,
    working_dir: &Path,
    config: &WorkflowConfig,
    agent_refs: &[AgentRef],
    session_sender: SessionEventSender,
    iteration: u32,
    session_logger: Arc<SessionLogger>,
    emit_round_started: bool,
    actor_ref: Option<ActorRef<WorkflowMessage>>,
) -> Result<ReviewBatchResult> {
    if agent_refs.is_empty() {
        anyhow::bail!("No reviewers configured");
    }

    let display_ids: Vec<&str> = agent_refs.iter().map(|r| r.display_id()).collect();

    // Conditionally output message based on whether this is sequential or parallel
    if agent_refs.len() == 1 {
        session_sender.send_output(format!("[review] Running reviewer: {}", display_ids[0]));
    } else {
        session_sender.send_output(format!(
            "[review] Running {} reviewer(s) in parallel: {}",
            agent_refs.len(),
            display_ids.join(", ")
        ));
    }

    // Only emit round started if caller requests it
    if emit_round_started {
        session_sender.send_review_round_started(ReviewKind::Plan, iteration);
    }

    // Check if tags are required from config
    let require_tags = config.workflow.reviewing.require_plan_feedback_tags;

    // Build agent contexts: (display_id, conversation_id, resume_strategy, custom_prompt)
    // Reviewing always uses Stateless - each review should be independent
    // with a fresh perspective on the plan, not influenced by prior sessions.
    let mut agent_contexts: Vec<(String, Option<String>, ResumeStrategy, Option<String>)> =
        Vec::new();
    for agent_ref in agent_refs {
        let display_id = agent_ref.display_id().to_string();
        let custom_prompt = agent_ref.custom_prompt().map(|s| s.to_string());

        // Use namespaced session key to avoid collisions with planning sessions
        let conversation_id_name = reviewing_conversation_key(&display_id);
        let agent_id = AgentId::from(conversation_id_name.as_str());

        // Look up existing conversation state from view, default to Stateless with no conversation
        let (conv_id, resume_strategy) = view
            .agent_conversations()
            .get(&agent_id)
            .map(|state| {
                (
                    state.conversation_id().map(|c| c.0.clone()),
                    state.resume_strategy(),
                )
            })
            .unwrap_or((None, ResumeStrategy::Stateless));

        agent_contexts.push((
            display_id.clone(),
            conv_id.clone(),
            resume_strategy,
            custom_prompt,
        ));

        // Dispatch RecordInvocation command to CQRS actor
        dispatch_reviewing_command(
            &actor_ref,
            &session_logger,
            DomainCommand::RecordInvocation {
                agent_id,
                phase: PhaseLabel::Reviewing,
                conversation_id: conv_id.clone().map(ConversationId::from),
                resume_strategy: to_domain_resume_strategy(&resume_strategy),
            },
        )
        .await;
    }

    // Build agents: (display_id, AgentType, conversation_id, resume_strategy, custom_prompt)
    #[allow(clippy::type_complexity)]
    let agents: Vec<(
        String,         // display_id
        AgentType,      // agent
        Option<String>, // conversation_id
        ResumeStrategy, // resume_strategy
        Option<String>, // custom_prompt
        String,         // skill_name
    )> = agent_refs
        .iter()
        .zip(agent_contexts.into_iter())
        .map(
            |(agent_ref, (display_id, conversation_id, resume_strategy, custom_prompt))| {
                let agent_name = agent_ref.agent_name();
                let agent_config = config.get_agent(agent_name).ok_or_else(|| {
                    anyhow::anyhow!("Review agent '{}' not found in config", agent_name)
                })?;
                let skill_name = agent_ref
                    .skill()
                    .unwrap_or(DEFAULT_REVIEW_SKILL)
                    .to_string();
                Ok((
                    display_id,
                    AgentType::from_config(agent_name, agent_config, working_dir.to_path_buf())?,
                    conversation_id,
                    resume_strategy,
                    custom_prompt,
                    skill_name,
                ))
            },
        )
        .collect::<Result<Vec<_>>>()?;

    // Get plan path (absolute)
    let plan_path = view
        .plan_path()
        .map(|p| p.0.clone())
        .unwrap_or_else(|| PathBuf::from("plan.md"));
    let plan_path_abs = if plan_path.is_absolute() {
        plan_path.clone()
    } else {
        working_dir.join(&plan_path)
    };

    // Get objective for prompts
    let objective = view.objective().map(|o| o.0.clone()).unwrap_or_default();
    let session_id = view
        .workflow_id()
        .map(|id| id.0.to_string())
        .unwrap_or_default();

    let futures: Vec<_> = agents
        .into_iter()
        .map(|(display_id, agent, conversation_id, resume_strategy, custom_prompt, skill_name)| {
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
                sender.send_reviewer_started(ReviewKind::Plan, iter, display_id.clone());

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
                    custom_prompt.as_deref(),
                    Some(&skill_name),
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
                            &parse_failure.error,
                            &initial_output,
                            &skill_name,
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

                session_sender
                    .send_output(format!("[review:{}] Verdict: {}", agent_name, verdict_str));

                // Use summary, with fallback to default if empty
                let summary = if review.summary.trim().is_empty() {
                    "Review completed".to_string()
                } else {
                    review.summary.clone()
                };

                // Send reviewer completed event with duration
                session_sender.send_reviewer_completed(
                    ReviewKind::Plan,
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
                    state_path: None,
                    plan_file: Some(&plan_path),
                    feedback_file: Some(&feedback_file_path),
                    workflow_session_id: Some(&session_id),
                };

                let bundle_path = create_review_bundle(bundle_config);

                if let Some(ref path) = bundle_path {
                    session_sender.send_output(format!(
                        "[review:{}] Diagnostics bundle created: {}",
                        agent_name,
                        path.display()
                    ));
                }

                let full_error = format!("Failed to parse review verdict after retry. {}", error);
                session_sender
                    .send_output(format!("[review:{}] ERROR: {}", agent_name, full_error));

                // Send reviewer failed event
                session_sender.send_reviewer_failed(
                    ReviewKind::Plan,
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
                session_sender
                    .send_output(format!("[error] {} review failed: {}", agent_name, error));

                // Send reviewer failed event
                session_sender.send_reviewer_failed(
                    ReviewKind::Plan,
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
) -> Result<SubmittedReview, crate::app::ParseFailureInfo> {
    // Check if the file exists
    if !feedback_path.exists() {
        return Err(crate::app::ParseFailureInfo {
            error: format!("Feedback file not found: {}", feedback_path.display()),
            plan_feedback_found: false,
            verdict_found: false,
        });
    }

    // Read the file content
    let content = match fs::read_to_string(feedback_path) {
        Ok(c) => c,
        Err(e) => {
            return Err(crate::app::ParseFailureInfo {
                error: format!("Failed to read feedback file: {}", e),
                plan_feedback_found: false,
                verdict_found: false,
            });
        }
    };

    if content.trim().is_empty() {
        return Err(crate::app::ParseFailureInfo {
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
        resume_strategy: *resume_strategy,
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
    fs::rename(&temp_path, path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to rename {} to {}: {}",
            temp_path.display(),
            path.display(),
            e
        )
    })?;
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

/// Helper to dispatch reviewing commands to the CQRS actor.
async fn dispatch_reviewing_command(
    actor_ref: &Option<ActorRef<WorkflowMessage>>,
    session_logger: &Arc<SessionLogger>,
    cmd: DomainCommand,
) {
    if let Some(ref actor) = actor_ref {
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Err(e) =
            actor.send_message(WorkflowMessage::Command(Box::new(cmd.clone()), reply_tx))
        {
            session_logger.log(
                LogLevel::Warn,
                LogCategory::Workflow,
                &format!("Failed to send reviewing command: {}", e),
            );
            return;
        }
        match reply_rx.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    &format!("Reviewing command rejected: {}", e),
                );
            }
            Err(_) => {
                session_logger.log(
                    LogLevel::Warn,
                    LogCategory::Workflow,
                    "Reviewing command reply channel closed",
                );
            }
        }
    }
}

/// Convert state ResumeStrategy to domain ResumeStrategy.
fn to_domain_resume_strategy(strategy: &ResumeStrategy) -> DomainResumeStrategy {
    match strategy {
        ResumeStrategy::Stateless => DomainResumeStrategy::Stateless,
        ResumeStrategy::ConversationResume => DomainResumeStrategy::ConversationResume,
        ResumeStrategy::ResumeLatest => DomainResumeStrategy::ResumeLatest,
    }
}

#[cfg(test)]
#[path = "tests/reviewing_tests.rs"]
mod tests;
