//! Review output parsing for file-based review feedback.
//!
//! This module handles parsing of review feedback content to extract structured review data,
//! including verdict extraction, summary parsing, and critical issues identification.

use crate::diagnostics::ParseFailureInfo;
use crate::phases::review_schema::{ReviewVerdict, SubmittedReview};
use regex::Regex;

/// Extract content from <plan-feedback> tags if present
pub fn extract_plan_feedback(output: &str) -> String {
    let re = Regex::new(r"(?s)<plan-feedback>\s*(.*?)\s*</plan-feedback>").unwrap();
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
pub fn parse_review_feedback(content: &str, require_tags: bool) -> Result<SubmittedReview, ParseFailureInfo> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_extract_plan_feedback_with_tags() {
        let output = "Some preamble\n<plan-feedback>\n## Summary\nThis is good\n</plan-feedback>\nEnd";
        let feedback = extract_plan_feedback(output);
        assert!(feedback.contains("## Summary"));
        assert!(feedback.contains("This is good"));
        assert!(!feedback.contains("preamble"));
    }

    #[test]
    fn test_extract_plan_feedback_without_tags() {
        let output = "## Summary\nThis is good\n\nNo tags here";
        let feedback = extract_plan_feedback(output);
        assert_eq!(feedback, output);
    }

    #[test]
    fn test_parse_review_feedback_without_tags_when_not_required() {
        let content = "## Summary\nLooks good!\n\n## Overall Assessment: APPROVED";
        let result = parse_review_feedback(content, false);
        assert!(result.is_ok());
        let review = result.unwrap();
        assert_eq!(review.verdict, ReviewVerdict::Approved);
    }

    #[test]
    fn test_parse_review_feedback_without_tags_when_required() {
        let content = "## Summary\nLooks good!\n\n## Overall Assessment: APPROVED";
        let result = parse_review_feedback(content, true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.error.contains("tags not found"));
    }

    #[test]
    fn test_parse_review_feedback_with_tags_when_required() {
        let content = "<plan-feedback>\n## Summary\nLooks good!\n\n## Overall Assessment: APPROVED\n</plan-feedback>";
        let result = parse_review_feedback(content, true);
        assert!(result.is_ok());
        let review = result.unwrap();
        assert_eq!(review.verdict, ReviewVerdict::Approved);
    }
}
