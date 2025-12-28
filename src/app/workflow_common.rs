use crate::phases::{ReviewFailure, ReviewResult};
use crate::state::State;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;

pub const REVIEW_FAILURE_RETRY_LIMIT: usize = 1;
pub const PLANNING_FAILURE_RETRY_LIMIT: usize = 2;

/// Pre-creates empty plan and feedback files before agent execution.
/// This ensures unique filenames are claimed before agents start writing.
/// Handles `AlreadyExists` as success for resumed workflows.
pub fn pre_create_plan_files(working_dir: &Path, state: &State) -> anyhow::Result<()> {
    let plan_path = working_dir.join(&state.plan_file);
    let feedback_path = working_dir.join(&state.feedback_file);

    // Pre-create plan file
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&plan_path)
    {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to pre-create plan file {}: {}",
                plan_path.display(),
                e
            ))
        }
    }

    // Pre-create feedback file
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&feedback_path)
    {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to pre-create feedback file {}: {}",
                feedback_path.display(),
                e
            ))
        }
    }

    Ok(())
}

/// Checks if a plan file has meaningful content (non-empty).
/// Use this instead of `path.exists()` for pre-created files.
pub fn plan_file_has_content(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.len() > 0,
        Err(_) => false,
    }
}

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

/// Deprecated: No longer called since we now keep old feedback files.
/// Kept for backward compatibility with tests but not used in production.
#[allow(dead_code)]
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

    #[test]
    fn test_plan_file_has_content_returns_false_for_nonexistent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.md");
        assert!(!plan_file_has_content(&path));
    }

    #[test]
    fn test_plan_file_has_content_returns_false_for_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.md");
        fs::write(&path, "").unwrap();
        assert!(!plan_file_has_content(&path));
    }

    #[test]
    fn test_plan_file_has_content_returns_true_for_file_with_content() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plan.md");
        fs::write(&path, "# Plan\n\nThis is a plan.").unwrap();
        assert!(plan_file_has_content(&path));
    }

    #[test]
    fn test_pre_create_plan_files_creates_new_files() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        // Create docs/plans directory
        fs::create_dir_all(working_dir.join("docs/plans")).unwrap();

        let state = State::new("test-feature", "Test objective", 3);
        let result = pre_create_plan_files(working_dir, &state);
        assert!(result.is_ok());

        // Verify files exist and are empty
        let plan_path = working_dir.join(&state.plan_file);
        let feedback_path = working_dir.join(&state.feedback_file);

        assert!(plan_path.exists(), "Plan file should exist");
        assert!(feedback_path.exists(), "Feedback file should exist");
        assert_eq!(fs::read_to_string(&plan_path).unwrap(), "");
        assert_eq!(fs::read_to_string(&feedback_path).unwrap(), "");
    }

    #[test]
    fn test_pre_create_plan_files_handles_already_exists() {
        let dir = tempdir().unwrap();
        let working_dir = dir.path();

        // Create docs/plans directory
        fs::create_dir_all(working_dir.join("docs/plans")).unwrap();

        let state = State::new("test-feature", "Test objective", 3);

        // Pre-create with existing content
        let plan_path = working_dir.join(&state.plan_file);
        let feedback_path = working_dir.join(&state.feedback_file);
        fs::write(&plan_path, "existing plan content").unwrap();
        fs::write(&feedback_path, "existing feedback content").unwrap();

        // Should succeed without error (AlreadyExists is handled)
        let result = pre_create_plan_files(working_dir, &state);
        assert!(result.is_ok());

        // Original content should be preserved (not overwritten)
        assert_eq!(fs::read_to_string(&plan_path).unwrap(), "existing plan content");
        assert_eq!(fs::read_to_string(&feedback_path).unwrap(), "existing feedback content");
    }
}
