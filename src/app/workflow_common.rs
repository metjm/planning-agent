use crate::domain::failure::NETWORK_ERROR_PATTERN;
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
#[path = "tests/workflow_common_tests.rs"]
mod tests;
