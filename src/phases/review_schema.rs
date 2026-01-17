//! Review schema types for file-based review workflow.
//!
//! This module contains the core types for structured review feedback,
//! used by the file-based review system.

use serde::{Deserialize, Serialize};

/// Structured review feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmittedReview {
    /// APPROVED or NEEDS_REVISION
    pub verdict: ReviewVerdict,
    /// One-paragraph summary of the review
    pub summary: String,
    /// List of critical/blocking issues (empty if approved)
    #[serde(default)]
    pub critical_issues: Vec<String>,
    /// List of recommendations (non-blocking)
    #[serde(default)]
    pub recommendations: Vec<String>,
    /// Full markdown feedback (optional, for detailed review)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_feedback: Option<String>,
}

impl SubmittedReview {
    /// Returns true if this review indicates the plan needs revision
    pub fn needs_revision(&self) -> bool {
        matches!(self.verdict, ReviewVerdict::NeedsRevision)
    }

    /// Returns the feedback content, preferring full_feedback if available
    pub fn feedback_content(&self) -> String {
        if let Some(ref full) = self.full_feedback {
            full.clone()
        } else {
            let mut content = format!("## Summary\n\n{}\n", self.summary);

            if !self.critical_issues.is_empty() {
                content.push_str("\n## Critical Issues\n\n");
                for issue in &self.critical_issues {
                    content.push_str(&format!("- {}\n", issue));
                }
            }

            if !self.recommendations.is_empty() {
                content.push_str("\n## Recommendations\n\n");
                for rec in &self.recommendations {
                    content.push_str(&format!("- {}\n", rec));
                }
            }

            content.push_str(&format!(
                "\n## Overall Assessment: {}\n",
                match self.verdict {
                    ReviewVerdict::Approved => "APPROVED",
                    ReviewVerdict::NeedsRevision => "NEEDS REVISION",
                }
            ));

            content
        }
    }
}

/// Review verdict enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewVerdict {
    Approved,
    NeedsRevision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_review_verdict_serialization() {
        let approved = ReviewVerdict::Approved;
        let json = serde_json::to_string(&approved).unwrap();
        assert_eq!(json, "\"APPROVED\"");

        let needs_revision = ReviewVerdict::NeedsRevision;
        let json = serde_json::to_string(&needs_revision).unwrap();
        assert_eq!(json, "\"NEEDS_REVISION\"");
    }

    #[test]
    fn test_review_verdict_deserialization() {
        let approved: ReviewVerdict = serde_json::from_str("\"APPROVED\"").unwrap();
        assert_eq!(approved, ReviewVerdict::Approved);

        let needs_revision: ReviewVerdict = serde_json::from_str("\"NEEDS_REVISION\"").unwrap();
        assert_eq!(needs_revision, ReviewVerdict::NeedsRevision);
    }

    #[test]
    fn test_submitted_review_needs_revision() {
        let approved = SubmittedReview {
            verdict: ReviewVerdict::Approved,
            summary: "Looks good".to_string(),
            critical_issues: vec![],
            recommendations: vec![],
            full_feedback: None,
        };
        assert!(!approved.needs_revision());

        let needs_rev = SubmittedReview {
            verdict: ReviewVerdict::NeedsRevision,
            summary: "Issues found".to_string(),
            critical_issues: vec!["Missing error handling".to_string()],
            recommendations: vec![],
            full_feedback: None,
        };
        assert!(needs_rev.needs_revision());
    }

    #[test]
    fn test_submitted_review_feedback_content() {
        let review = SubmittedReview {
            verdict: ReviewVerdict::NeedsRevision,
            summary: "The plan has some issues".to_string(),
            critical_issues: vec!["Issue 1".to_string(), "Issue 2".to_string()],
            recommendations: vec!["Suggestion 1".to_string()],
            full_feedback: None,
        };

        let content = review.feedback_content();
        assert!(content.contains("## Summary"));
        assert!(content.contains("The plan has some issues"));
        assert!(content.contains("## Critical Issues"));
        assert!(content.contains("- Issue 1"));
        assert!(content.contains("## Recommendations"));
        assert!(content.contains("NEEDS REVISION"));
    }

    #[test]
    fn test_submitted_review_full_feedback_takes_precedence() {
        let review = SubmittedReview {
            verdict: ReviewVerdict::Approved,
            summary: "Looks good".to_string(),
            critical_issues: vec![],
            recommendations: vec![],
            full_feedback: Some("# Custom Review\n\nFull custom content here.".to_string()),
        };

        let content = review.feedback_content();
        assert_eq!(content, "# Custom Review\n\nFull custom content here.");
    }
}
