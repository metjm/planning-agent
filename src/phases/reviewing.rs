use crate::agents::{AgentContext, AgentType};
use crate::config::{AggregationMode, WorkflowConfig};
use crate::state::{FeedbackStatus, ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn extract_plan_feedback(output: &str) -> String {
    let re = Regex::new(r"(?s)<plan-feedback>\s*(.*?)\s*</plan-feedback>").unwrap();
    if let Some(captures) = re.captures(output) {
        if let Some(content) = captures.get(1) {
            return content.as_str().to_string();
        }
    }

    output.to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerdictParseResult {
    Approved,
    NeedsRevision,
    ParseFailure(String),
}

pub fn parse_verdict(feedback: &str) -> VerdictParseResult {
    let re = Regex::new(r"(?i)overall\s+assessment[:\*\s]*\**\s*(APPROVED|NEEDS\s*_?\s*REVISION|MAJOR\s+ISSUES)")
        .unwrap();

    if let Some(captures) = re.captures(feedback) {
        if let Some(verdict_match) = captures.get(1) {
            let verdict = verdict_match.as_str().to_uppercase();
            let normalized = verdict.replace('_', " ").replace("  ", " ");

            if normalized == "APPROVED" {
                return VerdictParseResult::Approved;
            } else if normalized.contains("NEEDS") && normalized.contains("REVISION") {
                return VerdictParseResult::NeedsRevision;
            } else if normalized.contains("MAJOR") && normalized.contains("ISSUES") {
                return VerdictParseResult::NeedsRevision;
            }
        }
    }

    VerdictParseResult::ParseFailure("No valid Overall Assessment found".to_string())
}

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
}

#[derive(Debug, Clone)]
pub struct ReviewFailure {
    pub agent_name: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct ReviewBatchResult {
    pub reviews: Vec<ReviewResult>,
    pub failures: Vec<ReviewFailure>,
}

const REVIEW_SYSTEM_PROMPT: &str = r#"You are a technical plan reviewer.
Review the plan for correctness, completeness, and technical accuracy.
Use the "plan-review" skill to review.
"#;

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
        "[review] Running {} reviewer(s) in parallel: {}",
        agent_names.len(),
        agent_names.join(", ")
    ));

    let total_reviewers = agent_names.len();
    let base_feedback_path = state.feedback_file.clone();
    let base_feedback_path_abs = working_dir.join(&state.feedback_file);

    let phase_name = format!("Reviewing #{}", iteration);

    let mut agent_contexts: Vec<(String, Option<String>, ResumeStrategy)> = Vec::new();
    for agent_name in agent_names {
        let agent_session = state.get_or_create_agent_session(agent_name, ResumeStrategy::Stateless);
        agent_contexts.push((
            agent_name.clone(),
            agent_session.session_key.clone(),
            agent_session.resume_strategy.clone(),
        ));
        state.record_invocation(agent_name, &phase_name);
    }
    state.save_atomic(state_path)?;

    let agents: Vec<(String, AgentType, PathBuf, Option<String>, ResumeStrategy)> = agent_names
        .iter()
        .zip(agent_contexts.into_iter())
        .map(|(name, (_, session_key, resume_strategy))| {
            let agent_config = config
                .get_agent(name)
                .ok_or_else(|| anyhow::anyhow!("Review agent '{}' not found in config", name))?;
            let feedback_path =
                feedback_path_for_agent(&base_feedback_path, name, total_reviewers);
            Ok((
                name.to_string(),
                AgentType::from_config(name, agent_config, working_dir.to_path_buf())?,
                working_dir.join(feedback_path),
                session_key,
                resume_strategy,
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let futures: Vec<_> = agents
        .into_iter()
        .map(|(agent_name, agent, feedback_path_abs, session_key, resume_strategy)| {
            let p = build_review_prompt(state);
            let sender = session_sender.clone();
            let phase = format!("Reviewing #{}", iteration);

            async move {
                sender.send_output(format!("[review:{}] Starting review...", agent_name));
                let started_at = SystemTime::now();

                let context = AgentContext {
                    session_sender: sender.clone(),
                    phase,
                    session_key,
                    resume_strategy,
                };

                let result = agent
                    .execute_streaming_with_context(
                        p,
                        Some(REVIEW_SYSTEM_PROMPT.to_string()),
                        None,
                        context,
                    )
                    .await;

                sender.send_output(format!("[review:{}] Review complete", agent_name));
                (agent_name, feedback_path_abs, started_at, result)
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

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
                                if let Ok(Some(content)) = read_recent(&base_feedback_path_abs) {
                                    output = content;
                                    feedback_source = Some(base_feedback_path_abs.clone());
                                    session_sender.send_output(format!(
                                        "[review:{}] WARNING: feedback written to {} (expected {})",
                                        agent_name,
                                        base_feedback_path_abs.display(),
                                        feedback_path.display()
                                    ));
                                }
                            }
                        }
                    }
                }

                let extracted = extract_plan_feedback(&output);
                let feedback = if extracted != output {
                    session_sender.send_output(format!(
                        "[review:{}] Extracted feedback from <plan-feedback> tags",
                        agent_name
                    ));
                    extracted
                } else {
                    session_sender.send_output(format!(
                        "[review:{}] WARNING: No <plan-feedback> tags found, using raw output",
                        agent_name
                    ));
                    output
                };

                let trimmed_feedback = feedback.trim();
                if trimmed_feedback.is_empty() {
                    let error = "No feedback produced".to_string();
                    session_sender.send_output(format!(
                        "[review:{}] ERROR: {}",
                        agent_name, error
                    ));
                    failures.push(ReviewFailure { agent_name, error });
                    continue;
                }

                if let Some(source) = feedback_source {
                    session_sender.send_output(format!(
                        "[review:{}] Loaded feedback from {}",
                        agent_name,
                        source.display()
                    ));
                }

                if agent_result.is_error {
                    session_sender.send_output(format!(
                        "[review:{}] WARNING: reviewer reported an error; using available feedback",
                        agent_name
                    ));
                }

                let verdict = parse_verdict(&feedback);
                match verdict {
                    VerdictParseResult::Approved => {
                        session_sender.send_output(format!(
                            "[review:{}] Verdict: APPROVED",
                            agent_name
                        ));
                        reviews.push(ReviewResult {
                            agent_name,
                            needs_revision: false,
                            feedback,
                        });
                    }
                    VerdictParseResult::NeedsRevision => {
                        session_sender.send_output(format!(
                            "[review:{}] Verdict: NEEDS REVISION",
                            agent_name
                        ));
                        reviews.push(ReviewResult {
                            agent_name,
                            needs_revision: true,
                            feedback,
                        });
                    }
                    VerdictParseResult::ParseFailure(ref err) => {
                        session_sender.send_output(format!(
                            "[review:{}] WARNING: Could not parse verdict from feedback: {}",
                            agent_name, err
                        ));
                        failures.push(ReviewFailure {
                            agent_name,
                            error: format!("Verdict parse failure: {}", err),
                        });
                    }
                }
            }
            Err(e) => {

                session_sender.send_output(format!(
                    "[error] {} review failed: {}",
                    agent_name, e
                ));
                failures.push(ReviewFailure {
                    agent_name,
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(ReviewBatchResult { reviews, failures })
}

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

Read the plan file first, then provide your detailed review.

CRITICAL: You MUST wrap your final feedback in <plan-feedback> tags. Only the content inside these tags will be saved as the review feedback. Everything outside these tags (thinking, tool calls, intermediate steps) will be ignored.

Example format:
<plan-feedback>
## Review Summary
...your assessment here...

## Issues Found
...specific issues...

## Overall Assessment: APPROVED/NEEDS REVISION
</plan-feedback>"###,
        state.objective,
        state.plan_file.display()
    )
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

    #[test]
    fn test_parse_verdict_approved() {
        let feedback = "## Review Summary\nLooks good!\n\n## Overall Assessment:** APPROVED";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
    }

    #[test]
    fn test_parse_verdict_needs_revision() {
        let feedback = "## Issues Found\nSome problems.\n\n## Overall Assessment:** NEEDS REVISION";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
    }

    #[test]
    fn test_parse_verdict_needs_revision_underscore() {
        let feedback = "## Overall Assessment:** NEEDS_REVISION";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
    }

    #[test]
    fn test_parse_verdict_major_issues() {
        let feedback = "## Overall Assessment: MAJOR ISSUES\n\nSevere problems found.";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
    }

    #[test]
    fn test_parse_verdict_case_insensitive() {
        let feedback = "overall assessment: approved";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
    }

    #[test]
    fn test_parse_verdict_malformed_no_verdict() {
        let feedback = "## Overall Assessment:\nSome text but no verdict keyword.";
        assert!(matches!(
            parse_verdict(feedback),
            VerdictParseResult::ParseFailure(_)
        ));
    }

    #[test]
    fn test_parse_verdict_missing_overall_assessment() {
        let feedback = "This feedback has no overall assessment line at all.\nJust random content.";
        assert!(matches!(
            parse_verdict(feedback),
            VerdictParseResult::ParseFailure(_)
        ));
    }

    #[test]
    fn test_parse_verdict_conflicting_content() {
        let feedback = "The plan is APPROVED in some areas but has issues.\n\n## Overall Assessment:** NEEDS REVISION";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::NeedsRevision);
    }

    #[test]
    fn test_parse_verdict_no_major_issues_in_body() {
        let feedback = "I found no major issues in this plan.\n\n## Overall Assessment:** APPROVED";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
    }

    #[test]
    fn test_parse_verdict_with_markdown_formatting() {
        let feedback = "### Overall Assessment: **APPROVED**\n\nReady for implementation.";
        assert_eq!(parse_verdict(feedback), VerdictParseResult::Approved);
    }
}
