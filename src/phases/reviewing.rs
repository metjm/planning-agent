use crate::agents::AgentType;
use crate::claude::ClaudeInvocation;
use crate::config::{AggregationMode, WorkflowConfig};
use crate::state::{FeedbackStatus, State};
use crate::tui::Event;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::sync::mpsc;

const ALLOWED_TOOLS: &[&str] = &[
    "Read", "Glob", "Grep", "Write", "WebSearch", "WebFetch", "Skill", "Task",
];

/// Result from a single reviewer
#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
}

/// Result when a reviewer fails to produce a usable review
#[derive(Debug, Clone)]
pub struct ReviewFailure {
    pub agent_name: String,
    pub error: String,
}

/// Batch results for a multi-agent review run
#[derive(Debug, Clone)]
pub struct ReviewBatchResult {
    pub reviews: Vec<ReviewResult>,
    pub failures: Vec<ReviewFailure>,
}

const REVIEW_SYSTEM_PROMPT: &str = r#"You are a technical plan reviewer.
Review the plan for correctness, completeness, and technical accuracy.
Output "APPROVED" or "NEEDS REVISION" with specific feedback."#;

/// Run review phase with a single agent (legacy behavior)
pub async fn run_review_phase(
    state: &State,
    working_dir: &Path,
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<()> {
    let prompt = format!(
        r###"Use the Skill tool to invoke the plan-review skill:
Skill(skill: "plan-review", args: "{}")

User objective (used to create the plan):
```text
{}
```

Return the full feedback as markdown in your final response. Do not write files.

IMPORTANT: Your feedback MUST include one of these exact strings in the output:
- "Overall Assessment:** APPROVED" - if the plan is ready for implementation
- "Overall Assessment:** NEEDS REVISION" - if the plan has issues that need to be fixed

The orchestrator will parse your response to determine the next phase."###,
        state.plan_file.display(),
        state.objective,
    );

    let system_prompt = r#"You are orchestrating a plan review workflow.
Your task is to invoke the plan-review skill to review an implementation plan.
The review must result in a clear APPROVED or NEEDS REVISION assessment.
Do not ask questions - proceed with the skill invocation immediately."#;

    let result = ClaudeInvocation::new(prompt)
        .with_system_prompt(system_prompt)
        .with_allowed_tools(ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
        .with_working_dir(working_dir.to_path_buf())
        .execute_streaming(output_tx.clone())
        .await?;

    let feedback_path = working_dir.join(&state.feedback_file);
    let mut feedback = result.result;
    if feedback.trim().is_empty() && feedback_path.exists() {
        if let Ok(content) = fs::read_to_string(&feedback_path) {
            feedback = content;
        }
    }

    if feedback.trim().is_empty() {
        anyhow::bail!("Review produced no output");
    }

    if let Some(parent) = feedback_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&feedback_path, &feedback)?;

    let _ = output_tx.send(Event::Output("[planning-agent] Review phase complete".to_string()));
    let _ = output_tx.send(Event::Output(format!(
        "[planning-agent] Result preview: {}...",
        feedback.chars().take(200).collect::<String>()
    )));

    Ok(())
}

/// Run review phase with multiple agents in parallel
pub async fn run_multi_agent_review_phase(
    state: &State,
    working_dir: &Path,
    config: &WorkflowConfig,
    agent_names: &[String],
    output_tx: mpsc::UnboundedSender<Event>,
) -> Result<ReviewBatchResult> {
    if agent_names.is_empty() {
        anyhow::bail!("No reviewers configured");
    }

    let _ = output_tx.send(Event::Output(format!(
        "[review] Running {} reviewer(s) in parallel: {}",
        agent_names.len(),
        agent_names.join(", ")
    )));

    let total_reviewers = agent_names.len();
    let base_feedback_path = state.feedback_file.as_path();
    let base_feedback_path_abs = working_dir.join(&state.feedback_file);

    // Build agents from config
    let agents: Vec<(String, AgentType, PathBuf)> = agent_names
        .iter()
        .map(|name| {
            let agent_config = config
                .get_agent(name)
                .ok_or_else(|| anyhow::anyhow!("Review agent '{}' not found in config", name))?;
            let feedback_path =
                feedback_path_for_agent(base_feedback_path, name, total_reviewers);
            Ok((
                name.to_string(),
                AgentType::from_config(name, agent_config, working_dir.to_path_buf())?,
                working_dir.join(feedback_path),
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    // Execute all reviewers in parallel using futures::future::join_all
    let futures: Vec<_> = agents
        .into_iter()
        .map(|(agent_name, agent, feedback_path_abs)| {
            let p = build_review_prompt(state);
            let tx = output_tx.clone();

            async move {
                let _ = tx.send(Event::Output(format!(
                    "[review:{}] Starting review...",
                    agent_name
                )));
                let started_at = SystemTime::now();
                let result = agent
                    .execute_streaming(p, Some(REVIEW_SYSTEM_PROMPT.to_string()), None, tx.clone())
                    .await;
                let _ = tx.send(Event::Output(format!(
                    "[review:{}] Review complete",
                    agent_name
                )));
                (agent_name, feedback_path_abs, started_at, result)
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // Parse each result for APPROVED/NEEDS_REVISION
    let mut reviews = Vec::new();
    let mut failures = Vec::new();
    for (agent_name, feedback_path, started_at, result) in results {
        match result {
            Ok(agent_result) => {
                let mut output = agent_result.output;
                let mut feedback_source: Option<PathBuf> = None;

                if output.trim().is_empty() {
                    let read_recent = |path: &Path| -> Result<Option<String>> {
                        let metadata = fs::metadata(path)?;
                        let modified = metadata.modified()?;
                        if modified.duration_since(started_at).is_ok() {
                            Ok(Some(fs::read_to_string(path)?))
                        } else {
                            Ok(None)
                        }
                    };

                    match read_recent(&feedback_path) {
                        Ok(Some(content)) => {
                            output = content;
                            feedback_source = Some(feedback_path.clone());
                        }
                        Ok(None) | Err(_) => {
                            if feedback_path != base_feedback_path_abs {
                                match read_recent(&base_feedback_path_abs) {
                                    Ok(Some(content)) => {
                                        output = content;
                                        feedback_source = Some(base_feedback_path_abs.clone());
                                        let _ = output_tx.send(Event::Output(format!(
                                            "[review:{}] WARNING: feedback written to {} (expected {})",
                                            agent_name,
                                            base_feedback_path_abs.display(),
                                            feedback_path.display()
                                        )));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                let trimmed_output = output.trim();
                if trimmed_output.is_empty() {
                    let error = "No output produced".to_string();
                    let _ = output_tx.send(Event::Output(format!(
                        "[review:{}] ERROR: {}",
                        agent_name, error
                    )));
                    failures.push(ReviewFailure { agent_name, error });
                    continue;
                }

                if let Some(source) = feedback_source {
                    let _ = output_tx.send(Event::Output(format!(
                        "[review:{}] Loaded feedback from {}",
                        agent_name,
                        source.display()
                    )));
                }

                if agent_result.is_error {
                    let _ = output_tx.send(Event::Output(format!(
                        "[review:{}] WARNING: reviewer reported an error; using available feedback",
                        agent_name
                    )));
                }

                let needs_revision = output.contains("NEEDS REVISION")
                    || output.contains("NEEDS_REVISION")
                    || output.contains("MAJOR ISSUES");

                let _ = output_tx.send(Event::Output(format!(
                    "[review:{}] Verdict: {}",
                    agent_name,
                    if needs_revision {
                        "NEEDS REVISION"
                    } else {
                        "APPROVED"
                    }
                )));

                reviews.push(ReviewResult {
                    agent_name,
                    needs_revision,
                    feedback: output,
                });
            }
            Err(e) => {
                // Log error but continue with other reviewers
                let _ = output_tx.send(Event::Output(format!(
                    "[error] {} review failed: {}",
                    agent_name, e
                )));
                failures.push(ReviewFailure {
                    agent_name,
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(ReviewBatchResult { reviews, failures })
}

/// Build the review prompt for multi-agent review
fn build_review_prompt(state: &State) -> String {
    format!(
        r###"User objective (used to create the plan):
```text
{}
```

Review the implementation plan at: {}

IMPORTANT: Your feedback MUST include one of these exact strings in the output:
- "Overall Assessment:** APPROVED" - if the plan is ready for implementation
- "Overall Assessment:** NEEDS REVISION" - if the plan has issues that need to be fixed

Provide your assessment with one of these verdicts:
- "APPROVED" - if the plan is ready for implementation
- "NEEDS REVISION" - if the plan has issues that need to be fixed

Include specific feedback about any issues found.

Return the full feedback as markdown in your final response. Do not write files.

Read the plan file first, then provide your detailed review."###,
        state.objective,
        state.plan_file.display()
    )
}

/// Aggregate reviews based on configured aggregation mode
pub fn aggregate_reviews(reviews: &[ReviewResult], mode: &AggregationMode) -> FeedbackStatus {
    if reviews.is_empty() {
        return FeedbackStatus::NeedsRevision; // No reviews = problem
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

/// Write individual feedback files for each reviewer
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

/// Merge multiple reviewer feedback into a single file
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
            },
            ReviewResult {
                agent_name: "gemini".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
            },
        ];
        // 1 of 3 rejects = 33%, not majority
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
            },
            ReviewResult {
                agent_name: "codex".to_string(),
                needs_revision: true,
                feedback: "NEEDS REVISION".to_string(),
            },
            ReviewResult {
                agent_name: "gemini".to_string(),
                needs_revision: false,
                feedback: "APPROVED".to_string(),
            },
        ];
        // 2 of 3 rejects = 66%, majority
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
