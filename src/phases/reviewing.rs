use crate::agents::{AgentContext, AgentType};
use crate::app::failure::FailureKind;
use crate::app::workflow_common::is_network_error;
use crate::config::{AggregationMode, WorkflowConfig};
use crate::diagnostics::{create_mcp_review_bundle, AttemptTimestamp, BundleConfig};
use crate::mcp::spawner::generate_mcp_server_config;
use crate::mcp::{McpServerConfig, SubmittedReview};
use crate::phases::review_parser::parse_mcp_review;
use crate::phases::review_prompts::{
    build_mcp_agent_prompt, build_mcp_recovery_prompt, build_mcp_review_prompt, REVIEW_SYSTEM_PROMPT,
};
use crate::state::{FeedbackStatus, ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

// Re-export VerdictParseResult and parse_verdict for external use (used in tests)
#[allow(unused_imports)]
pub use crate::phases::review_parser::{parse_verdict, VerdictParseResult};

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
    /// Short summary of the review (from MCP or extracted from feedback)
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
        mcp_server_name: String,
    },
    /// Agent execution error (not a parse failure)
    ExecutionError(String),
}

pub async fn run_multi_agent_review_with_context(
    state: &mut State,
    working_dir: &Path,
    config: &WorkflowConfig,
    agent_names: &[String],
    session_sender: SessionEventSender,
    iteration: u32,
    state_path: &Path,
) -> Result<ReviewBatchResult> {
    if agent_names.is_empty() {
        anyhow::bail!("No reviewers configured");
    }

    session_sender.send_output(format!(
        "[review] Running {} reviewer(s) in parallel with MCP: {}",
        agent_names.len(),
        agent_names.join(", ")
    ));

    let phase_name = format!("Reviewing #{}", iteration);

    let mut agent_contexts: Vec<(String, Option<String>, ResumeStrategy)> = Vec::new();
    for agent_name in agent_names {
        // Get the configured resume strategy for this agent
        let configured_strategy = config
            .get_agent(agent_name)
            .map(|cfg| {
                if cfg.session_persistence.enabled {
                    cfg.session_persistence.strategy.clone()
                } else {
                    ResumeStrategy::Stateless
                }
            })
            .unwrap_or(ResumeStrategy::Stateless);
        let agent_session = state.get_or_create_agent_session(agent_name, configured_strategy);
        agent_contexts.push((
            agent_name.clone(),
            agent_session.session_key.clone(),
            agent_session.resume_strategy.clone(),
        ));
        state.record_invocation(agent_name, &phase_name);
    }
    state.set_updated_at();
    state.save_atomic(state_path)?;

    let agents: Vec<(String, AgentType, Option<String>, ResumeStrategy)> = agent_names
        .iter()
        .zip(agent_contexts.into_iter())
        .map(|(name, (_, session_key, resume_strategy))| {
            let agent_config = config
                .get_agent(name)
                .ok_or_else(|| anyhow::anyhow!("Review agent '{}' not found in config", name))?;
            Ok((
                name.to_string(),
                AgentType::from_config(name, agent_config, working_dir.to_path_buf())?,
                session_key,
                resume_strategy,
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    // Read plan content for MCP server
    let plan_path = state.plan_file.clone();
    let plan_content = fs::read_to_string(&plan_path)?;

    // Pre-build prompts outside the closure to avoid borrow issues
    let mcp_review_prompt = build_mcp_review_prompt(state, working_dir);
    let mcp_agent_prompt = build_mcp_agent_prompt(state, working_dir);

    let futures: Vec<_> = agents
        .into_iter()
        .map(|(agent_name, agent, session_key, resume_strategy)| {
            let plan = plan_content.clone();
            let review_prompt = mcp_review_prompt.clone();
            let mcp_prompt = mcp_agent_prompt.clone();
            let sender = session_sender.clone();
            let phase = format!("Reviewing #{}", iteration);

            async move {
                sender.send_output(format!("[review:{}] Starting MCP review...", agent_name));

                // Generate MCP config for this agent
                let mcp_config = match generate_mcp_server_config(&plan, &review_prompt) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        return (
                            agent_name,
                            ReviewExecutionResult::ExecutionError(format!(
                                "MCP config injection failed: {}",
                                e
                            )),
                        );
                    }
                };

                // Execute agent with MCP - all agents use MCP, no fallbacks
                sender.send_output(format!(
                    "[review:{}] Using MCP for structured feedback (server: {})",
                    agent_name, mcp_config.server_name
                ));

                // First attempt
                let attempt1_result = execute_review_attempt(
                    &agent,
                    &mcp_prompt,
                    &session_key,
                    &resume_strategy,
                    &sender,
                    &phase,
                    &mcp_config,
                )
                .await;

                let (initial_output, attempt1_timestamp) = match attempt1_result {
                    Ok(result) => (result.output, AttemptTimestamp {
                        attempt: 1,
                        started_at: result.started_at,
                        ended_at: result.ended_at,
                    }),
                    Err(e) => {
                        return (
                            agent_name,
                            ReviewExecutionResult::ExecutionError(e.to_string()),
                        );
                    }
                };

                if initial_output.trim().is_empty() {
                    return (
                        agent_name,
                        ReviewExecutionResult::ExecutionError("No output from reviewer".to_string()),
                    );
                }

                // Try to parse the initial output
                match parse_mcp_review(&initial_output) {
                    Ok(review) => {
                        sender.send_output(format!("[review:{}] Review complete", agent_name));
                        (agent_name, ReviewExecutionResult::Success(review))
                    }
                    Err(parse_failure) => {
                        // Initial attempt failed to parse - try recovery
                        sender.send_output(format!(
                            "[review:{}] Failed to parse verdict: {}. Attempting recovery...",
                            agent_name, parse_failure.error
                        ));

                        // Build recovery prompt with context
                        let recovery_prompt = build_mcp_recovery_prompt(
                            &mcp_config.server_name,
                            &initial_output,
                            &parse_failure.error,
                        );

                        // Execute retry attempt
                        let attempt2_result = execute_review_attempt(
                            &agent,
                            &recovery_prompt,
                            &session_key,
                            &resume_strategy,
                            &sender,
                            &format!("{} (recovery)", phase),
                            &mcp_config,
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
                                    agent_name, e
                                ));
                                (None, None)
                            }
                        };

                        // Try to parse retry output
                        if let Some(ref retry_out) = retry_output {
                            if !retry_out.trim().is_empty() {
                                if let Ok(review) = parse_mcp_review(retry_out) {
                                    sender.send_output(format!(
                                        "[review:{}] Recovery succeeded!",
                                        agent_name
                                    ));
                                    return (agent_name, ReviewExecutionResult::Success(review));
                                }
                            }
                        }

                        // Both attempts failed - prepare for bundle creation
                        let mut timestamps = vec![attempt1_timestamp];
                        if let Some(ts) = attempt2_timestamp {
                            timestamps.push(ts);
                        }

                        // Get the most recent parse failure info
                        let final_parse_result = retry_output
                            .as_ref()
                            .and_then(|out| parse_mcp_review(out).err())
                            .unwrap_or(parse_failure);

                        sender.send_output(format!(
                            "[review:{}] Recovery failed. Diagnostics bundle will be created.",
                            agent_name
                        ));

                        (
                            agent_name,
                            ReviewExecutionResult::ParseFailure {
                                error: final_parse_result.error,
                                plan_feedback_found: final_parse_result.plan_feedback_found,
                                verdict_found: final_parse_result.verdict_found,
                                initial_output,
                                retry_output,
                                attempt_timestamps: timestamps,
                                mcp_server_name: mcp_config.server_name.clone(),
                            },
                        )
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

    for (agent_name, result) in results {
        match result {
            ReviewExecutionResult::Success(mcp_review) => {
                let needs_revision = mcp_review.needs_revision();
                let feedback = mcp_review.feedback_content();

                let verdict_str = if needs_revision {
                    "NEEDS REVISION"
                } else {
                    "APPROVED"
                };

                session_sender.send_output(format!(
                    "[review:{}] Verdict: {} (via MCP)",
                    agent_name, verdict_str
                ));

                // Use MCP summary, with fallback to default if empty
                let summary = if mcp_review.summary.trim().is_empty() {
                    "Review completed".to_string()
                } else {
                    mcp_review.summary.clone()
                };

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
                mcp_server_name,
            } => {
                // Create diagnostics bundle
                let bundle_config = BundleConfig {
                    working_dir,
                    agent_name: &agent_name,
                    failure_reason: &error,
                    mcp_server_name: &mcp_server_name,
                    run_id: &run_id,
                    plan_feedback_found,
                    verdict_found,
                    attempt_timestamps,
                    initial_output: Some(&initial_output),
                    retry_output: retry_output.as_deref(),
                    state_path: Some(state_path),
                    plan_file: Some(&state.plan_file),
                    feedback_file: Some(&state.feedback_file),
                    workflow_session_id: Some(&state.workflow_session_id),
                };

                let bundle_path = create_mcp_review_bundle(bundle_config);

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

                failures.push(ReviewFailure {
                    agent_name,
                    error: full_error.clone(),
                    bundle_path,
                    kind: FailureKind::ParseFailure(full_error),
                });
            }
            ReviewExecutionResult::ExecutionError(error) => {
                session_sender.send_output(format!("[error] {} review failed: {}", agent_name, error));
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

/// Execute a single review attempt and track timing
async fn execute_review_attempt(
    agent: &AgentType,
    prompt: &str,
    session_key: &Option<String>,
    resume_strategy: &ResumeStrategy,
    sender: &SessionEventSender,
    phase: &str,
    mcp_config: &McpServerConfig,
) -> Result<ReviewAttemptResult> {
    let started_at = chrono::Utc::now().to_rfc3339();

    let context = AgentContext {
        session_sender: sender.clone(),
        phase: phase.to_string(),
        session_key: session_key.clone(),
        resume_strategy: resume_strategy.clone(),
    };

    let result = agent
        .execute_streaming_with_mcp(
            prompt.to_string(),
            Some(REVIEW_SYSTEM_PROMPT.to_string()),
            None,
            context,
            mcp_config,
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

pub fn write_feedback_files(
    reviews: &[ReviewResult],
    base_feedback_path: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    let mut paths = Vec::new();

    for review in reviews {
        let feedback_path =
            feedback_path_for_agent(base_feedback_path, &review.agent_name, reviews.len());
        std::fs::write(&feedback_path, &review.feedback)?;
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

    std::fs::write(output_path, format!("{}{}", header, merged))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_any_rejects_none() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "Plan looks good".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "No issues found".to_string(),
            },
        ];
        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
            FeedbackStatus::Approved
        );
    }

    #[test]
    fn test_aggregate_any_rejects_one() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "Plan looks good".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Missing error handling".to_string(),
            },
        ];
        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
            FeedbackStatus::NeedsRevision
        );
    }

    #[test]
    fn test_aggregate_all_reject_partial() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "Plan looks good".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Missing error handling".to_string(),
            },
        ];
        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::AllReject),
            FeedbackStatus::Approved
        );
    }

    #[test]
    fn test_aggregate_all_reject_full() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Architecture concerns".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Missing error handling".to_string(),
            },
        ];
        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::AllReject),
            FeedbackStatus::NeedsRevision
        );
    }

    #[test]
    fn test_aggregate_majority_one_of_three() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "Plan looks good".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "No issues found".to_string(),
            },
            ReviewResult {
                agent_name: "gemini".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Minor issues found".to_string(),
            },
        ];

        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::Majority),
            FeedbackStatus::Approved
        );
    }

    #[test]
    fn test_aggregate_majority_two_of_three() {
        let reviews = vec![
            ReviewResult {
                agent_name: "claude".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Architecture concerns".to_string(),
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
                summary: "Missing error handling".to_string(),
            },
            ReviewResult {
                agent_name: "gemini".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
                summary: "Plan looks good".to_string(),
            },
        ];

        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::Majority),
            FeedbackStatus::NeedsRevision
        );
    }

    #[test]
    fn test_aggregate_empty_reviews() {
        let reviews: Vec<ReviewResult> = vec![];
        assert_eq!(
            aggregate_reviews(&reviews, &AggregationMode::AnyRejects),
            FeedbackStatus::NeedsRevision
        );
    }
}
