use crate::domain::failure::NETWORK_ERROR_PATTERN;
use crate::domain::input::NewWorkflowInput;
use crate::domain::types::{Phase, WorkflowId};
use crate::planning_paths::{session_dir, SessionInfo};
use regex::Regex;
use std::path::Path;

/// Creates the session folder, optionally recording a working directory.
///
/// The working directory is stored in session_info.json for session listing.
/// Note: Plan file is created by AI agent, not pre-created empty.
/// Note: Feedback files are created by individual reviewers during the reviewing phase.
pub fn pre_create_session_folder_with_working_dir(
    input: &NewWorkflowInput,
    workflow_id: &WorkflowId,
    working_dir: Option<&Path>,
) -> anyhow::Result<()> {
    // Create the session folder (session_dir already creates directories)
    let _folder = session_dir(&workflow_id.to_string())?;

    // Note: Plan file is created by AI agent with actual content.
    // Note: Feedback files are created by individual reviewers.

    // Create session_info.json for fast listing
    let default_wd = std::env::current_dir().unwrap_or_default();
    let wd = working_dir.unwrap_or(&default_wd);
    let session_info = SessionInfo::new(
        &workflow_id.to_string(),
        input.feature_name.as_str(),
        input.objective.as_str(),
        wd,
        &format!("{:?}", Phase::Planning),
        1, // Initial iteration
    );
    if let Err(e) = session_info.save(&workflow_id.to_string()) {
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
#[path = "tests/workflow_common_tests.rs"]
mod tests;
