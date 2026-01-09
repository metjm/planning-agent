use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Structured review feedback submitted via MCP
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

/// JSON Schema for the submit_review tool
pub fn submit_review_schema() -> Value {
    json!({
        "type": "object",
        "required": ["verdict", "summary"],
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["APPROVED", "NEEDS_REVISION"],
                "description": "Your final verdict on the plan. Use APPROVED if the plan is ready for implementation, or NEEDS_REVISION if there are issues that must be fixed."
            },
            "summary": {
                "type": "string",
                "description": "One-paragraph summary of your review assessment."
            },
            "critical_issues": {
                "type": "array",
                "items": { "type": "string" },
                "description": "List of blocking issues that must be fixed before the plan can be approved. Leave empty if verdict is APPROVED."
            },
            "recommendations": {
                "type": "array",
                "items": { "type": "string" },
                "description": "List of non-blocking suggestions for improvement."
            },
            "full_feedback": {
                "type": "string",
                "description": "Full markdown review content with detailed analysis. Optional - if not provided, feedback will be generated from summary and issues."
            }
        }
    })
}

/// JSON Schema for the get_plan tool
pub fn get_plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
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

    #[test]
    fn test_submit_review_schema_valid() {
        let schema = submit_review_schema();
        assert!(schema.get("type").is_some());
        assert!(schema.get("required").is_some());
        assert!(schema.get("properties").is_some());
    }
}
