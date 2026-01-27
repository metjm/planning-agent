//! Review output parsing for file-based review feedback.
//!
//! This module handles parsing of review feedback content to extract structured review data,
//! including verdict extraction, summary parsing, and critical issues identification.

use crate::app::ParseFailureInfo;
use crate::phases::review_schema::{ReviewVerdict, SubmittedReview};
use regex::Regex;

/// Extract content from <plan-feedback> tags if present
pub fn extract_plan_feedback(output: &str) -> String {
    let re = Regex::new(r"(?s)<plan-feedback>\s*(.*?)\s*</plan-feedback>")
        .expect("regex to extract content between <plan-feedback> tags");
    if let Some(captures) = re.captures(output) {
        if let Some(content) = captures.get(1) {
            return content.as_str().to_string();
        }
    }

    output.to_string()
}

/// Parse review feedback content, returning detailed failure info on error.
///
/// # Arguments
/// * `content` - The feedback file content to parse
/// * `require_tags` - If true, requires <plan-feedback> tags to be present
pub fn parse_review_feedback(
    content: &str,
    require_tags: bool,
) -> Result<SubmittedReview, ParseFailureInfo> {
    // First, check if plan-feedback tags are present
    let has_tags = content.contains("<plan-feedback>") && content.contains("</plan-feedback>");

    // If tags are required but not found, return error
    if require_tags && !has_tags {
        return Err(ParseFailureInfo {
            error: "Required <plan-feedback> tags not found in feedback content".to_string(),
            plan_feedback_found: false,
            verdict_found: false,
        });
    }

    // Extract content from tags if present
    let feedback = extract_plan_feedback(content);
    let plan_feedback_found = has_tags;

    // Try to parse as JSON (if agent returned structured output)
    if let Ok(review) = serde_json::from_str::<SubmittedReview>(&feedback) {
        return Ok(review);
    }

    // Otherwise, parse the verdict from the feedback text and construct SubmittedReview
    let verdict_result = parse_verdict(&feedback);
    let verdict_found = !matches!(verdict_result, VerdictParseResult::ParseFailure(_));

    match verdict_result {
        VerdictParseResult::Approved => Ok(SubmittedReview {
            verdict: ReviewVerdict::Approved,
            summary: extract_summary_from_feedback(&feedback),
            critical_issues: vec![],
            recommendations: extract_recommendations_from_feedback(&feedback),
            full_feedback: Some(feedback),
        }),
        VerdictParseResult::NeedsRevision => Ok(SubmittedReview {
            verdict: ReviewVerdict::NeedsRevision,
            summary: extract_summary_from_feedback(&feedback),
            critical_issues: extract_critical_issues_from_feedback(&feedback),
            recommendations: extract_recommendations_from_feedback(&feedback),
            full_feedback: Some(feedback),
        }),
        VerdictParseResult::ParseFailure(error) => Err(ParseFailureInfo {
            error,
            plan_feedback_found,
            verdict_found,
        }),
    }
}

/// Extract a summary from feedback text
fn extract_summary_from_feedback(feedback: &str) -> String {
    // Try to find a summary section
    let summary_re = Regex::new(
        r"(?is)##?\s*(?:summary|review summary|executive summary)[:\s]*\n+(.*?)(?:\n\n|\n##|\z)",
    )
    .expect("regex to match summary/review summary/executive summary section headings");
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
    let issues_re = Regex::new(r"(?is)##?\s*(?:critical\s+issues?|blocking\s+issues?|major\s+issues?)[:\s]*\n+(.*?)(?:\n##|\z)")
        .expect("regex to match critical/blocking/major issues section headings");
    if let Some(captures) = issues_re.captures(feedback) {
        if let Some(content) = captures.get(1) {
            for line in content.as_str().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("•")
                {
                    let issue = trimmed
                        .trim_start_matches(['-', '*', '•', ' '].as_ref())
                        .trim();
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
    let recs_re = Regex::new(
        r"(?is)##?\s*(?:recommendations?|suggestions?|improvements?)[:\s]*\n+(.*?)(?:\n##|\z)",
    )
    .expect("regex to match recommendations/suggestions/improvements section headings");
    if let Some(captures) = recs_re.captures(feedback) {
        if let Some(content) = captures.get(1) {
            for line in content.as_str().lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with("•")
                {
                    let rec = trimmed
                        .trim_start_matches(['-', '*', '•', ' '].as_ref())
                        .trim();
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
    let re = Regex::new(
        r"(?i)overall\s+assessment[:\*\s]*\**\s*(APPROVED|NEEDS\s*_?\s*REVISION|MAJOR\s+ISSUES)",
    )
    .expect("regex to match overall assessment verdict (APPROVED/NEEDS_REVISION/MAJOR ISSUES)");

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

#[cfg(test)]
#[path = "tests/review_parser_tests.rs"]
mod tests;
