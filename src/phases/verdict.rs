//! Verdict parsing for implementation-review phase.
//!
//! This module provides verdict parsing used by the implementation-review phase.

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Result of parsing a verification/review verdict from a report.
///
/// Used by both verification and implementation-review phases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VerificationVerdictResult {
    Approved,
    NeedsRevision,
    ParseFailure { reason: String },
}

impl VerificationVerdictResult {
    /// Returns true if the verdict indicates revision is needed.
    pub fn needs_revision(&self) -> bool {
        matches!(
            self,
            VerificationVerdictResult::NeedsRevision
                | VerificationVerdictResult::ParseFailure { .. }
        )
    }
}

/// Parses the verification verdict from a verification/review report.
///
/// Looks for "Verdict: APPROVED" or "Verdict: NEEDS REVISION" patterns.
/// Supports various formatting styles including markdown headers, colons, and bold markers.
pub fn parse_verification_verdict(report: &str) -> VerificationVerdictResult {
    let re =
        Regex::new(r"(?i)(?:##\s*)?Verdict[:\*\s]*\**\s*(APPROVED|NEEDS\s*_?\s*REVISION)").unwrap();

    if let Some(captures) = re.captures(report) {
        if let Some(verdict_match) = captures.get(1) {
            let verdict = verdict_match.as_str().to_uppercase();
            let normalized = verdict.replace('_', " ").replace("  ", " ");

            if normalized == "APPROVED" {
                return VerificationVerdictResult::Approved;
            } else if normalized.contains("NEEDS") && normalized.contains("REVISION") {
                return VerificationVerdictResult::NeedsRevision;
            }
        }
    }

    VerificationVerdictResult::ParseFailure {
        reason: "No valid Verdict found in report".to_string(),
    }
}

/// Extracts feedback content from a specified XML-style tag.
///
/// This function is generic over the tag name to support both
/// `<verification-feedback>` and `<implementation-feedback>` tags.
///
/// # Arguments
/// * `tag` - The tag name (without angle brackets) to extract content from
/// * `report` - The report text to search
///
/// # Returns
/// The content between the opening and closing tags, or None if not found.
pub fn extract_feedback_tag(tag: &str, report: &str) -> Option<String> {
    let pattern = format!(r"(?s)<{}>\s*(.*?)\s*</{}>", tag, tag);
    let re = Regex::new(&pattern).unwrap();
    if let Some(captures) = re.captures(report) {
        if let Some(content) = captures.get(1) {
            return Some(content.as_str().to_string());
        }
    }
    None
}

/// Extracts feedback content from `<implementation-feedback>` tags.
///
/// Convenience wrapper around `extract_feedback_tag` for implementation-review reports.
pub fn extract_implementation_feedback(report: &str) -> Option<String> {
    extract_feedback_tag("implementation-feedback", report)
}

#[cfg(test)]
#[path = "tests/verdict_tests.rs"]
mod tests;
