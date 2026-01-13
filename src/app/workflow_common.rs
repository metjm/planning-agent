use crate::app::failure::{FailureKind, NETWORK_ERROR_PATTERN};
use crate::phases::{ReviewFailure, ReviewResult};
use crate::state::State;
use regex::Regex;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

/// Default retry limit for review failures.
/// Can be overridden via config.failure_policy.max_retries.
#[allow(dead_code)]
pub const DEFAULT_REVIEW_FAILURE_RETRY_LIMIT: usize = 2;

/// Pre-creates the plan folder and empty plan/feedback files before agent execution.
/// Plan files are stored in ~/.planning-agent/plans/<folder>/ so paths are absolute.
/// Handles `AlreadyExists` as success for resumed workflows.
pub fn pre_create_plan_files(state: &State) -> anyhow::Result<()> {
    let plan_path = &state.plan_file;
    let feedback_path = &state.feedback_file;

    // Create the plan folder (parent directory of plan file)
    if let Some(plan_folder) = plan_path.parent() {
        fs::create_dir_all(plan_folder).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create plan folder {}: {}",
                plan_folder.display(),
                e
            )
        })?;
    }

    // Pre-create plan file
    match OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(plan_path)
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
        .open(feedback_path)
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

#[allow(dead_code)]
pub fn should_retry_review(
    attempt: usize,
    failures: &[ReviewFailure],
    successful_reviews: &[ReviewResult],
    max_retries: usize,
) -> bool {
    !failures.is_empty() && successful_reviews.is_empty() && attempt < max_retries
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

/// Classifies a failure from stderr content and exit code.
///
/// Priority order:
/// 1. Timeout (explicit from caller)
/// 2. Network (stderr pattern match)
/// 3. ProcessExit (non-zero exit code)
/// 4. Unknown (fallback)
#[allow(dead_code)]
pub fn classify_failure_from_stderr(
    stderr: &str,
    exit_code: Option<i32>,
    is_timeout: bool,
) -> FailureKind {
    if is_timeout {
        return FailureKind::Timeout;
    }

    // Check for network error patterns
    if is_network_error(stderr) {
        return FailureKind::Network;
    }

    // Check for non-zero exit code
    if let Some(code) = exit_code {
        if code != 0 {
            return FailureKind::ProcessExit(code);
        }
    }

    // Fallback to unknown with stderr content for debugging
    FailureKind::Unknown(stderr.chars().take(500).collect())
}

/// Checks if stderr contains network error patterns.
#[allow(dead_code)]
pub fn is_network_error(stderr: &str) -> bool {
    // Use lazy_static pattern for compiled regex, but for simplicity
    // we'll compile on each call (acceptable for error paths)
    match Regex::new(NETWORK_ERROR_PATTERN) {
        Ok(re) => re.is_match(stderr),
        Err(_) => {
            // Fallback to simple substring matching
            let lower = stderr.to_lowercase();
            lower.contains("connect")
                || lower.contains("network")
                || lower.contains("econnrefused")
                || lower.contains("etimedout")
                || lower.contains("connection refused")
                || lower.contains("dns")
                || lower.contains("socket")
        }
    }
}

/// Calculates the backoff duration for a given retry attempt.
///
/// Uses exponential backoff: base_secs * 2^retry_count, capped at 5 minutes.
#[allow(dead_code)]
pub fn calculate_backoff(retry_count: u32, base_secs: u32) -> Duration {
    let max_backoff_secs = 300u64; // 5 minutes cap
    let backoff_secs = (base_secs as u64).saturating_mul(2u64.saturating_pow(retry_count));
    Duration::from_secs(backoff_secs.min(max_backoff_secs))
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
            bundle_path: None,
            kind: FailureKind::Network,
        }];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(should_retry_review(0, &failures, &reviews, 2));
    }

    #[test]
    fn test_should_retry_review_at_limit() {
        let failures = vec![ReviewFailure {
            agent_name: "test".to_string(),
            error: "error".to_string(),
            bundle_path: None,
            kind: FailureKind::Timeout,
        }];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(!should_retry_review(2, &failures, &reviews, 2));
    }

    #[test]
    fn test_should_retry_review_with_successful_reviews() {
        let failures = vec![ReviewFailure {
            agent_name: "test".to_string(),
            error: "error".to_string(),
            bundle_path: None,
            kind: FailureKind::ProcessExit(1),
        }];
        let reviews = vec![ReviewResult {
            agent_name: "claude".to_string(),
            needs_revision: false,
            feedback: "APPROVED".to_string(),
            summary: "Plan looks good".to_string(),
        }];
        assert!(!should_retry_review(0, &failures, &reviews, 2));
    }

    #[test]
    fn test_should_retry_review_no_failures() {
        let failures: Vec<ReviewFailure> = vec![];
        let reviews: Vec<ReviewResult> = vec![];
        assert!(!should_retry_review(0, &failures, &reviews, 2));
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
    fn test_pre_create_plan_files_creates_folder_and_files() {
        // State::new creates paths in ~/.planning-agent/plans/ which are absolute
        let state = State::new("test-feature", "Test objective", 3).unwrap();

        // The paths should be absolute (starting with home dir)
        assert!(state.plan_file.is_absolute() || state.plan_file.to_string_lossy().starts_with("/"));

        let result = pre_create_plan_files(&state);
        assert!(result.is_ok(), "pre_create_plan_files should succeed: {:?}", result);

        // Verify files exist and are empty
        assert!(state.plan_file.exists(), "Plan file should exist at {}", state.plan_file.display());
        assert!(state.feedback_file.exists(), "Feedback file should exist at {}", state.feedback_file.display());
        assert_eq!(fs::read_to_string(&state.plan_file).unwrap(), "");
        assert_eq!(fs::read_to_string(&state.feedback_file).unwrap(), "");

        // Cleanup
        if let Some(parent) = state.plan_file.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn test_pre_create_plan_files_handles_already_exists() {
        let state = State::new("test-feature-exists", "Test objective", 3).unwrap();

        // First, create the folder and files with content
        if let Some(parent) = state.plan_file.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&state.plan_file, "existing plan content").unwrap();
        fs::write(&state.feedback_file, "existing feedback content").unwrap();

        // Should succeed without error (AlreadyExists is handled)
        let result = pre_create_plan_files(&state);
        assert!(result.is_ok());

        // Original content should be preserved (not overwritten)
        assert_eq!(fs::read_to_string(&state.plan_file).unwrap(), "existing plan content");
        assert_eq!(fs::read_to_string(&state.feedback_file).unwrap(), "existing feedback content");

        // Cleanup
        if let Some(parent) = state.plan_file.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn test_classify_failure_timeout() {
        let result = classify_failure_from_stderr("some output", Some(0), true);
        assert_eq!(result, FailureKind::Timeout);
    }

    #[test]
    fn test_classify_failure_network_errors() {
        // Test various network error patterns
        let patterns = [
            "Error: connect ECONNREFUSED 127.0.0.1:443",
            "network error: failed to resolve host",
            "ETIMEDOUT waiting for response",
            "connection refused by server",
            "name resolution failed",
            "DNS lookup failed",
            "socket error: broken pipe",
        ];

        for pattern in patterns {
            let result = classify_failure_from_stderr(pattern, Some(1), false);
            assert_eq!(
                result,
                FailureKind::Network,
                "Pattern '{}' should be classified as Network",
                pattern
            );
        }
    }

    #[test]
    fn test_classify_failure_process_exit() {
        let result = classify_failure_from_stderr("some generic error", Some(42), false);
        assert_eq!(result, FailureKind::ProcessExit(42));
    }

    #[test]
    fn test_classify_failure_unknown() {
        let result = classify_failure_from_stderr("something unusual happened", None, false);
        match result {
            FailureKind::Unknown(msg) => assert!(msg.contains("something unusual")),
            other => panic!("Expected Unknown, got {:?}", other),
        }
    }

    #[test]
    fn test_is_network_error() {
        assert!(is_network_error("connect ECONNREFUSED"));
        assert!(is_network_error("NETWORK error"));
        assert!(is_network_error("DNS resolution failed"));
        assert!(!is_network_error("syntax error in code"));
        assert!(!is_network_error("file not found"));
    }

    #[test]
    fn test_calculate_backoff() {
        // Base: 5 seconds
        assert_eq!(calculate_backoff(0, 5), Duration::from_secs(5));  // 5 * 2^0 = 5
        assert_eq!(calculate_backoff(1, 5), Duration::from_secs(10)); // 5 * 2^1 = 10
        assert_eq!(calculate_backoff(2, 5), Duration::from_secs(20)); // 5 * 2^2 = 20
        assert_eq!(calculate_backoff(3, 5), Duration::from_secs(40)); // 5 * 2^3 = 40
    }

    #[test]
    fn test_calculate_backoff_cap() {
        // Should be capped at 5 minutes (300 seconds)
        assert_eq!(calculate_backoff(10, 5), Duration::from_secs(300));
        assert_eq!(calculate_backoff(20, 5), Duration::from_secs(300));
    }
}
