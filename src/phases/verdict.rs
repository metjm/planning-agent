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

    /// Converts the verdict to a string representation for state storage.
    pub fn to_state_string(&self) -> String {
        match self {
            VerificationVerdictResult::Approved => "APPROVED".to_string(),
            VerificationVerdictResult::NeedsRevision => "NEEDS_REVISION".to_string(),
            VerificationVerdictResult::ParseFailure { reason } => {
                format!("PARSE_FAILURE: {}", reason)
            }
        }
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
mod tests {
    use super::*;

    #[test]
    fn test_parse_verdict_approved() {
        let report = "## Verdict\nAPPROVED\n\nAll requirements met.";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_approved_with_colon() {
        let report = "## Verdict: APPROVED";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_needs_revision() {
        let report = "## Verdict\nNEEDS REVISION\n\nSome issues found.";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::NeedsRevision
        );
    }

    #[test]
    fn test_parse_verdict_needs_revision_underscore() {
        let report = "## Verdict: NEEDS_REVISION";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::NeedsRevision
        );
    }

    #[test]
    fn test_parse_verdict_case_insensitive() {
        let report = "Verdict: approved";
        assert_eq!(
            parse_verification_verdict(report),
            VerificationVerdictResult::Approved
        );
    }

    #[test]
    fn test_parse_verdict_missing() {
        let report = "This report has no verdict section.";
        assert!(matches!(
            parse_verification_verdict(report),
            VerificationVerdictResult::ParseFailure { .. }
        ));
    }

    #[test]
    fn test_extract_implementation_feedback() {
        let report = r#"
## Verdict
NEEDS REVISION

<implementation-feedback>
The config loading function needs error handling.
</implementation-feedback>
"#;
        let feedback = extract_implementation_feedback(report).unwrap();
        assert!(feedback.contains("config loading"));
        assert!(feedback.contains("error handling"));
    }

    #[test]
    fn test_extract_feedback_tag_custom() {
        let report = r#"
<custom-tag>
Some custom content here.
</custom-tag>
"#;
        let feedback = extract_feedback_tag("custom-tag", report).unwrap();
        assert!(feedback.contains("custom content"));
    }

    #[test]
    fn test_verdict_result_serialization() {
        let approved = VerificationVerdictResult::Approved;
        let json = serde_json::to_string(&approved).unwrap();
        assert!(json.contains("approved"));

        let parsed: VerificationVerdictResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, approved);

        let needs_revision = VerificationVerdictResult::NeedsRevision;
        let json = serde_json::to_string(&needs_revision).unwrap();
        assert!(json.contains("needs_revision"));

        let parse_failure = VerificationVerdictResult::ParseFailure {
            reason: "test error".to_string(),
        };
        let json = serde_json::to_string(&parse_failure).unwrap();
        assert!(json.contains("parse_failure"));
        assert!(json.contains("test error"));
    }
}
