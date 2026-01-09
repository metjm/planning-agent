use crate::agents::{AgentContext, AgentType};
use crate::config::{AggregationMode, WorkflowConfig};
use crate::mcp::spawner::generate_mcp_config;
use crate::mcp::{ReviewVerdict, SubmittedReview};
use crate::state::{FeedbackStatus, ResumeStrategy, State};
use crate::tui::SessionEventSender;
use anyhow::Result;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

fn extract_plan_feedback(output: &str) -> String {
    let re = Regex::new(r"(?s)<plan-feedback>\s*(.*?)\s*</plan-feedback>").unwrap();
    if let Some(captures) = re.captures(output) {
        if let Some(content) = captures.get(1) {
            return content.as_str().to_string();
        }
    }

    output.to_string()
}

/// Try to parse a structured review from agent output
/// Looks for JSON-like structured review or parses from <plan-feedback> tags
fn try_parse_mcp_review(output: &str) -> Option<SubmittedReview> {
    // First, try to extract from <plan-feedback> tags
    let feedback = extract_plan_feedback(output);

    // Try to parse as JSON (if agent returned structured output)
    if let Ok(review) = serde_json::from_str::<SubmittedReview>(&feedback) {
        return Some(review);
    }

    // Otherwise, parse the verdict from the feedback text and construct SubmittedReview
    let verdict_result = parse_verdict(&feedback);
    match verdict_result {
        VerdictParseResult::Approved => Some(SubmittedReview {
            verdict: ReviewVerdict::Approved,
            summary: extract_summary_from_feedback(&feedback),
            critical_issues: vec![],
            recommendations: extract_recommendations_from_feedback(&feedback),
            full_feedback: Some(feedback),
        }),
        VerdictParseResult::NeedsRevision => Some(SubmittedReview {
            verdict: ReviewVerdict::NeedsRevision,
            summary: extract_summary_from_feedback(&feedback),
            critical_issues: extract_critical_issues_from_feedback(&feedback),
            recommendations: extract_recommendations_from_feedback(&feedback),
            full_feedback: Some(feedback),
        }),
        VerdictParseResult::ParseFailure(_) => None,
    }
}

/// Extract a summary from feedback text
fn extract_summary_from_feedback(feedback: &str) -> String {
    // Try to find a summary section
    let summary_re = Regex::new(r"(?is)##?\s*(?:summary|review summary|executive summary)[:\s]*\n+(.*?)(?:\n\n|\n##|\z)").unwrap();
    if let Some(captures) = summary_re.captures(feedback) {
        if let Some(content) = captures.get(1) {
            let summary = content.as_str().trim();
            if !summary.is_empty() {
                return summary.to_string();
            }
        }
    }

    // Fall back to first paragraph
    feedback
        .lines()
        .find(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .unwrap_or("Review completed")
        .trim()
        .to_string()
}

/// Extract critical issues from feedback text
fn extract_critical_issues_from_feedback(feedback: &str) -> Vec<String> {
    let mut issues = vec![];

    // Look for critical issues section
    let issues_re = Regex::new(r"(?is)##?\s*(?:critical\s+issues?|blocking\s+issues?|major\s+issues?)[:\s]*\n+(.*?)(?:\n##|\z)").unwrap();
    if let Some(captures) = issues_re.captures(feedback) {
        if let Some(content) = captures.get(1) {
            for line in content.as_str().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("•") {
                    let issue = trimmed.trim_start_matches(['-', '*', '•', ' '].as_ref()).trim();
                    if !issue.is_empty() {
                        issues.push(issue.to_string());
                    }
                }
            }
        }
    }

    issues
}

/// Extract recommendations from feedback text
fn extract_recommendations_from_feedback(feedback: &str) -> Vec<String> {
    let mut recs = vec![];

    // Look for recommendations section
    let recs_re = Regex::new(r"(?is)##?\s*(?:recommendations?|suggestions?|improvements?)[:\s]*\n+(.*?)(?:\n##|\z)").unwrap();
    if let Some(captures) = recs_re.captures(feedback) {
        if let Some(content) = captures.get(1) {
            for line in content.as_str().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("•") {
                    let rec = trimmed.trim_start_matches(['-', '*', '•', ' '].as_ref()).trim();
                    if !rec.is_empty() {
                        recs.push(rec.to_string());
                    }
                }
            }
        }
    }

    recs
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
            } else if (normalized.contains("NEEDS") && normalized.contains("REVISION"))
                || (normalized.contains("MAJOR") && normalized.contains("ISSUES"))
            {
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
    let objective = state.objective.clone();
    let base_feedback_path = state.feedback_file.clone();
    let total_reviewers = agent_names.len();

    // Pre-build prompts outside the closure to avoid borrow issues
    let mcp_review_prompt = build_mcp_review_prompt(state);
    let mcp_agent_prompt = build_mcp_agent_prompt(state);

    let futures: Vec<_> = agents
        .into_iter()
        .map(|(agent_name, agent, session_key, resume_strategy)| {
            let plan = plan_content.clone();
            let review_prompt = mcp_review_prompt.clone();
            let mcp_prompt = mcp_agent_prompt.clone();
            let objective = objective.clone();
            let plan_path = plan_path.clone();
            let base_feedback_path = base_feedback_path.clone();
            let sender = session_sender.clone();
            let phase = format!("Reviewing #{}", iteration);

            async move {
                sender.send_output(format!("[review:{}] Starting MCP review...", agent_name));

                let context = AgentContext {
                    session_sender: sender.clone(),
                    phase,
                    session_key,
                    resume_strategy,
                };

                // Generate MCP config for this agent
                let mcp_config = match generate_mcp_config(&plan, &review_prompt) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        return (
                            agent_name,
                            Err(anyhow::anyhow!("Failed to generate MCP config: {}", e)),
                        );
                    }
                };

                let prompt = if agent.supports_mcp() {
                    mcp_prompt.clone()
                } else {
                    let feedback_path = feedback_path_for_agent(
                        &base_feedback_path,
                        &agent_name,
                        total_reviewers,
                    );
                    build_review_prompt_for_agent(&objective, &plan_path, &feedback_path)
                };

                // Execute agent with MCP config (Claude supports it, others fall back)
                let result = if agent.supports_mcp() {
                    sender.send_output(format!(
                        "[review:{}] Using MCP for structured feedback",
                        agent_name
                    ));
                    agent
                        .execute_streaming_with_mcp(
                            prompt,
                            Some(REVIEW_SYSTEM_PROMPT.to_string()),
                            None,
                            context,
                            &mcp_config,
                        )
                        .await
                } else {
                    sender.send_output(format!(
                        "[review:{}] Agent does not support MCP, using standard review",
                        agent_name
                    ));
                    agent
                        .execute_streaming_with_context(
                            prompt,
                            Some(REVIEW_SYSTEM_PROMPT.to_string()),
                            None,
                            context,
                        )
                        .await
                };

                sender.send_output(format!("[review:{}] Review complete", agent_name));
                (agent_name, result)
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    let mut reviews = Vec::new();
    let mut failures = Vec::new();

    for (agent_name, result) in results {
        match result {
            Ok(agent_result) => {
                let output = agent_result.output;

                if output.trim().is_empty() {
                    let error = "No output from reviewer".to_string();
                    session_sender.send_output(format!(
                        "[review:{}] ERROR: {}",
                        agent_name, error
                    ));
                    failures.push(ReviewFailure { agent_name, error });
                    continue;
                }

                if agent_result.is_error {
                    session_sender.send_output(format!(
                        "[review:{}] WARNING: reviewer reported an error; attempting to parse output",
                        agent_name
                    ));
                }

                // Try to parse MCP review from output
                if let Some(mcp_review) = try_parse_mcp_review(&output) {
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

                    reviews.push(ReviewResult {
                        agent_name,
                        needs_revision,
                        feedback,
                    });
                } else {
                    // Failed to parse review - this is an error with MCP-only mode
                    let error = "Failed to parse review verdict from output. Agent must use submit_review MCP tool or include 'Overall Assessment: APPROVED/NEEDS REVISION' in output.".to_string();
                    session_sender.send_output(format!(
                        "[review:{}] ERROR: {}",
                        agent_name, error
                    ));
                    failures.push(ReviewFailure { agent_name, error });
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

/// Build the review prompt that will be embedded in the MCP server's get_plan response
fn build_mcp_review_prompt(state: &State) -> String {
    format!(
        r###"User objective (used to create the plan):
```text
{}
```

Please review the implementation plan above for:
1. Technical correctness and feasibility
2. Completeness (does it address all requirements?)
3. Potential risks or issues
4. Code quality and best practices

After your review, you MUST submit your feedback using the `submit_review` MCP tool with:
- verdict: "APPROVED" or "NEEDS_REVISION"
- summary: A brief one-paragraph summary
- critical_issues: Array of blocking issues (if any)
- recommendations: Array of non-blocking suggestions"###,
        state.objective
    )
}

/// Build the prompt that instructs the agent to use the MCP tools
fn build_mcp_agent_prompt(state: &State) -> String {
    format!(
        r###"You are reviewing an implementation plan. Follow these steps:

1. Use the `get_plan` MCP tool to retrieve the plan content and review instructions
2. Read and analyze the plan thoroughly
3. Submit your review using the `submit_review` MCP tool

User objective: {}

IMPORTANT: You MUST use the MCP tools to complete this review:
- First call `get_plan` to get the plan content
- Then call `submit_review` with your verdict and feedback

After submitting your review via MCP, wrap your final assessment in <plan-feedback> tags:

<plan-feedback>
## Summary
[Your review summary]

## Critical Issues
[List any blocking issues, or "None" if approved]

## Recommendations
[Non-blocking suggestions]

## Overall Assessment: [APPROVED or NEEDS REVISION]
</plan-feedback>"###,
        state.objective
    )
}

fn build_review_prompt_for_agent(
    objective: &str,
    plan_path_abs: &Path,
    feedback_path_abs: &Path,
) -> String {
    format!(
        r###"User objective (used to create the plan):
```text
{}
```

Review the implementation plan at: {}

Write your feedback to: {}

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
        objective,
        plan_path_abs.display(),
        feedback_path_abs.display()
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

    #[test]
    fn test_build_review_prompt_for_agent() {
        let prompt = build_review_prompt_for_agent(
            "Ship the feature safely",
            Path::new("/tmp/plan.md"),
            Path::new("/tmp/feedback.md"),
        );

        assert!(prompt.contains("Ship the feature safely"));
        assert!(prompt.contains("/tmp/plan.md"));
        assert!(prompt.contains("/tmp/feedback.md"));
        assert!(prompt.contains("<plan-feedback>"));
        assert!(!prompt.contains("submit_review"));
        assert!(!prompt.contains("get_plan"));
    }
}
