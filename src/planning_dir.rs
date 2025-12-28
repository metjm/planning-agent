//! Centralized helper for `.planning-agent` directory creation and gitignore management.
//!
//! This module provides a single function to ensure the `.planning-agent` directory exists
//! and, when inside a Git repository, adds `.planning-agent/` to the repo's `.gitignore`
//! in an idempotent way.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The name of the planning agent directory.
pub const PLANNING_DIR_NAME: &str = ".planning-agent";

/// Ensures the `.planning-agent` directory exists within `working_dir`.
///
/// If the working directory is inside a Git repository, this function also
/// adds `.planning-agent/` to the repository's `.gitignore` file (creating
/// the file if it doesn't exist) in an idempotent way.
///
/// # Arguments
///
/// * `working_dir` - The directory that should contain `.planning-agent`.
///
/// # Returns
///
/// The path to the `.planning-agent` directory on success.
///
/// # Errors
///
/// Returns an error if directory creation fails. Git/gitignore operations
/// fail gracefully (logged but don't cause errors).
pub fn ensure_planning_agent_dir(working_dir: &Path) -> std::io::Result<PathBuf> {
    let planning_dir = working_dir.join(PLANNING_DIR_NAME);

    // Create the directory if it doesn't exist
    fs::create_dir_all(&planning_dir)?;

    // Attempt to update gitignore (failures are non-fatal)
    if let Err(e) = update_gitignore_if_in_repo(working_dir) {
        // Log warning but don't fail the operation
        eprintln!(
            "[planning-agent] Warning: Failed to update .gitignore: {}",
            e
        );
    }

    Ok(planning_dir)
}

/// Checks if git is available and we're inside a git repository.
/// If so, updates the repository's .gitignore to include `.planning-agent/`.
fn update_gitignore_if_in_repo(working_dir: &Path) -> Result<(), GitIgnoreError> {
    // Check if git is available
    if which::which("git").is_err() {
        return Ok(()); // No git, nothing to do
    }

    // Check if we're inside a git repository and get the repo root
    let repo_root = get_git_repo_root(working_dir)?;

    // Update the .gitignore at the repo root
    update_gitignore(&repo_root)
}

/// Error type for gitignore operations
#[derive(Debug)]
enum GitIgnoreError {
    NotInRepo,
    GitCommandFailed(String),
    IoError(std::io::Error),
}

impl std::fmt::Display for GitIgnoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitIgnoreError::NotInRepo => write!(f, "Not inside a git repository"),
            GitIgnoreError::GitCommandFailed(msg) => write!(f, "Git command failed: {}", msg),
            GitIgnoreError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl From<std::io::Error> for GitIgnoreError {
    fn from(e: std::io::Error) -> Self {
        GitIgnoreError::IoError(e)
    }
}

/// Gets the root directory of the git repository containing `working_dir`.
fn get_git_repo_root(working_dir: &Path) -> Result<PathBuf, GitIgnoreError> {
    let output = Command::new("git")
        .args(["-C", &working_dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| GitIgnoreError::GitCommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(GitIgnoreError::NotInRepo);
    }

    let root = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    if root.is_empty() {
        return Err(GitIgnoreError::NotInRepo);
    }

    Ok(PathBuf::from(root))
}

/// Updates the .gitignore file at `repo_root` to include `.planning-agent/`.
fn update_gitignore(repo_root: &Path) -> Result<(), GitIgnoreError> {
    let gitignore_path = repo_root.join(".gitignore");

    // Read existing content (or empty string if file doesn't exist)
    let content = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    // Check if any variant of the pattern already exists
    if has_planning_agent_pattern(&content) {
        return Ok(()); // Already in gitignore
    }

    // Append the pattern with proper newline handling
    let new_content = append_pattern(&content, ".planning-agent/");

    fs::write(&gitignore_path, new_content)?;

    Ok(())
}

/// Checks if the content already contains a pattern that ignores `.planning-agent`.
///
/// Recognizes patterns like:
/// - `.planning-agent`
/// - `.planning-agent/`
/// - `/.planning-agent/`
/// - `/.planning-agent`
fn has_planning_agent_pattern(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Normalize: remove leading `/` and trailing `/`
        let normalized = trimmed
            .trim_start_matches('/')
            .trim_end_matches('/');
        if normalized == ".planning-agent" {
            return true;
        }
    }
    false
}

/// Appends the pattern to the content with proper newline handling.
fn append_pattern(content: &str, pattern: &str) -> String {
    let mut result = content.to_string();

    // Ensure the file ends with a newline before appending
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }

    result.push_str(pattern);
    result.push('\n');

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper to initialize a git repo in a temp directory
    fn init_git_repo(dir: &Path) -> bool {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn test_creates_directory_and_updates_gitignore() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Initialize git repo
        if !init_git_repo(working_dir) {
            eprintln!("Git not available, skipping test");
            return;
        }

        // Call the helper
        let result = ensure_planning_agent_dir(working_dir);
        assert!(result.is_ok());

        // Verify directory was created
        let planning_dir = working_dir.join(PLANNING_DIR_NAME);
        assert!(planning_dir.exists());
        assert!(planning_dir.is_dir());

        // Verify gitignore was updated
        let gitignore_path = working_dir.join(".gitignore");
        assert!(gitignore_path.exists());
        let content = fs::read_to_string(&gitignore_path).unwrap();
        assert!(content.contains(".planning-agent/"));
    }

    #[test]
    fn test_creates_gitignore_if_missing() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Initialize git repo without .gitignore
        if !init_git_repo(working_dir) {
            eprintln!("Git not available, skipping test");
            return;
        }

        // Ensure no .gitignore exists
        let gitignore_path = working_dir.join(".gitignore");
        assert!(!gitignore_path.exists());

        // Call the helper
        let _ = ensure_planning_agent_dir(working_dir);

        // Verify gitignore was created
        assert!(gitignore_path.exists());
        let content = fs::read_to_string(&gitignore_path).unwrap();
        assert_eq!(content.trim(), ".planning-agent/");
    }

    #[test]
    fn test_idempotent_gitignore_update() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Initialize git repo
        if !init_git_repo(working_dir) {
            eprintln!("Git not available, skipping test");
            return;
        }

        // Call the helper twice
        let _ = ensure_planning_agent_dir(working_dir);
        let _ = ensure_planning_agent_dir(working_dir);

        // Verify only one entry in gitignore
        let gitignore_path = working_dir.join(".gitignore");
        let content = fs::read_to_string(&gitignore_path).unwrap();
        let count = content
            .lines()
            .filter(|l| l.contains(".planning-agent"))
            .count();
        assert_eq!(count, 1, "Should have exactly one .planning-agent entry");
    }

    #[test]
    fn test_handles_existing_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Initialize git repo
        if !init_git_repo(working_dir) {
            eprintln!("Git not available, skipping test");
            return;
        }

        // Test various existing patterns
        let patterns = [
            ".planning-agent",
            ".planning-agent/",
            "/.planning-agent/",
            "/.planning-agent",
        ];

        for pattern in patterns {
            // Create gitignore with existing pattern
            let gitignore_path = working_dir.join(".gitignore");
            fs::write(&gitignore_path, format!("{}\n", pattern)).unwrap();

            // Call the helper
            let _ = ensure_planning_agent_dir(working_dir);

            // Verify no duplicate entry
            let content = fs::read_to_string(&gitignore_path).unwrap();
            let count = content
                .lines()
                .filter(|l| {
                    let trimmed = l.trim();
                    !trimmed.is_empty()
                        && !trimmed.starts_with('#')
                        && trimmed
                            .trim_start_matches('/')
                            .trim_end_matches('/')
                            == ".planning-agent"
                })
                .count();
            assert_eq!(
                count, 1,
                "Pattern '{}' should not cause duplicate entry",
                pattern
            );
        }
    }

    #[test]
    fn test_no_git_repo() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Don't initialize git repo

        // Call the helper
        let result = ensure_planning_agent_dir(working_dir);
        assert!(result.is_ok());

        // Verify directory was created
        let planning_dir = working_dir.join(PLANNING_DIR_NAME);
        assert!(planning_dir.exists());

        // Verify no gitignore was created
        let gitignore_path = working_dir.join(".gitignore");
        assert!(!gitignore_path.exists());
    }

    #[test]
    fn test_newline_handling() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Initialize git repo
        if !init_git_repo(working_dir) {
            eprintln!("Git not available, skipping test");
            return;
        }

        // Create gitignore without trailing newline
        let gitignore_path = working_dir.join(".gitignore");
        fs::write(&gitignore_path, "node_modules").unwrap(); // No trailing newline

        // Call the helper
        let _ = ensure_planning_agent_dir(working_dir);

        // Verify proper formatting
        let content = fs::read_to_string(&gitignore_path).unwrap();
        assert!(content.contains("node_modules\n.planning-agent/\n"));
    }

    #[test]
    fn test_has_planning_agent_pattern() {
        assert!(has_planning_agent_pattern(".planning-agent\n"));
        assert!(has_planning_agent_pattern(".planning-agent/\n"));
        assert!(has_planning_agent_pattern("/.planning-agent/\n"));
        assert!(has_planning_agent_pattern("/.planning-agent\n"));
        assert!(has_planning_agent_pattern("  .planning-agent  \n"));
        assert!(has_planning_agent_pattern("node_modules\n.planning-agent/\ndist"));

        assert!(!has_planning_agent_pattern(""));
        assert!(!has_planning_agent_pattern("# .planning-agent\n"));
        assert!(!has_planning_agent_pattern("node_modules\n"));
        assert!(!has_planning_agent_pattern(".planning-agent-other\n"));
    }

    #[test]
    fn test_append_pattern() {
        // Empty content
        assert_eq!(append_pattern("", "test/"), "test/\n");

        // Content with trailing newline
        assert_eq!(append_pattern("foo\n", "test/"), "foo\ntest/\n");

        // Content without trailing newline
        assert_eq!(append_pattern("foo", "test/"), "foo\ntest/\n");
    }
}
