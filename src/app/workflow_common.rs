use crate::phases::{ReviewFailure, ReviewResult};
use std::path::Path;

pub const REVIEW_FAILURE_RETRY_LIMIT: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ReviewLoopOutcome {
    Approved,
    NeedsRevision,
    MaxIterationsReached,
}

#[allow(dead_code)]
pub fn should_retry_review(
    attempt: usize,
    failures: &[ReviewFailure],
    successful_reviews: &[ReviewResult],
) -> bool {
    !failures.is_empty() && successful_reviews.is_empty() && attempt < REVIEW_FAILURE_RETRY_LIMIT
}

pub fn cleanup_merged_feedback(feedback_path: &Path) -> Result<bool, std::io::Error> {
    if feedback_path.exists() {
        std::fs::remove_file(feedback_path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_should_retry_review_with_failures_and_empty_reviews() {
        let failures = vec![ReviewFailure {
            agent_name: "test".to_string(),
            error: "error".to_string(),
        }];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(should_retry_review(0, &failures, &reviews));
    }

    #[test]
    fn test_should_retry_review_at_limit() {
        let failures = vec![ReviewFailure {
            agent_name: "test".to_string(),
            error: "error".to_string(),
        }];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(!should_retry_review(REVIEW_FAILURE_RETRY_LIMIT, &failures, &reviews));
    }

    #[test]
    fn test_should_retry_review_with_successful_reviews() {
        let failures = vec![ReviewFailure {
            agent_name: "test".to_string(),
            error: "error".to_string(),
        }];
        let reviews = vec![ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
        }];
        assert!(!should_retry_review(0, &failures, &reviews));
    }

    #[test]
    fn test_should_retry_review_no_failures() {
        let failures: Vec<ReviewFailure> = vec![];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(!should_retry_review(0, &failures, &reviews));
    }

    #[test]
    fn test_cleanup_merged_feedback_removes_existing_file() {
        let dir = tempdir().unwrap();
        let feedback_path = dir.path().join("feedback.md");
        fs::write(&feedback_path, "test content").unwrap();

        let result = cleanup_merged_feedback(&feedback_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert!(!feedback_path.exists());
    }

    #[test]
    fn test_cleanup_merged_feedback_handles_missing_file() {
        let dir = tempdir().unwrap();
        let feedback_path = dir.path().join("nonexistent.md");

        let result = cleanup_merged_feedback(&feedback_path);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
