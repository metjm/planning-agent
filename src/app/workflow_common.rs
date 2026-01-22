use crate::app::failure::NETWORK_ERROR_PATTERN;
use crate::planning_paths::SessionInfo;
use crate::state::State;
use regex::Regex;
use std::fs;
use std::path::Path;

/// Creates the session folder, optionally recording a working directory.
///
/// The working directory is stored in session_info.json for session listing.
/// Note: Plan file is created by AI agent, not pre-created empty.
/// Note: Feedback files are created by individual reviewers during the reviewing phase.
pub fn pre_create_session_folder_with_working_dir(
    state: &State,
    working_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let plan_path = &state.plan_file;

    // Create the session folder (parent directory of plan file)
    if let Some(session_folder) = plan_path.parent() {
        fs::create_dir_all(session_folder).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create session folder {}: {}",
                session_folder.display(),
                e
            )
        })?;
    }

    // Note: Plan file is created by AI agent with actual content.
    // Note: Feedback files are created by individual reviewers.

    // Create session_info.json for fast listing
    let default_wd = std::env::current_dir().unwrap_or_default();
    let wd = working_dir.unwrap_or(&default_wd);
    let session_info = SessionInfo::new(
        &state.workflow_session_id,
        &state.feature_name,
        &state.objective,
        wd,
        &format!("{:?}", state.phase),
        state.iteration,
    );
    if let Err(e) = session_info.save(&state.workflow_session_id) {
        // Log warning but don't fail - session_info is optional metadata
        eprintln!(
            "[planning] Warning: Failed to save session_info.json: {}",
            e
        );
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

/// Checks if stderr contains network error patterns.
pub fn is_network_error(stderr: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

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
    fn test_pre_create_session_folder_with_working_dir() {
        let state = State::new("test-feature-wd", "Test with working dir", 3).unwrap();
        let temp_dir = tempdir().unwrap();
        let working_dir = temp_dir.path();

        let result = pre_create_session_folder_with_working_dir(&state, Some(working_dir));
        assert!(
            result.is_ok(),
            "pre_create_session_folder_with_working_dir should succeed: {:?}",
            result
        );

        // Verify session_info.json was created with correct working_dir
        let info = SessionInfo::load(&state.workflow_session_id);
        assert!(info.is_ok());
        let info = info.unwrap();
        assert_eq!(info.working_dir, working_dir);
        assert_eq!(info.feature_name, "test-feature-wd");

        // Cleanup
        if let Some(parent) = state.plan_file.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}
