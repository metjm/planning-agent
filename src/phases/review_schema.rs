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
#[path = "tests/review_schema_tests.rs"]
mod tests;
