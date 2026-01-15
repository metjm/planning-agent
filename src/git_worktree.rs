//! Git worktree management for isolated session workspaces.
//!
//! This module provides functionality to create and manage git worktrees,
//! allowing each planning session to work in an isolated branch without
//! affecting the user's main working directory.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Information about a git repository.
struct GitRepoInfo {
    /// Path to the repository root
    repo_root: PathBuf,
    /// Current branch name (None if detached HEAD)
    current_branch: Option<String>,
}

/// Information about a created worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    /// Path to the worktree directory
    pub worktree_path: PathBuf,
    /// Branch name created for this worktree
    pub branch_name: String,
    /// The branch that was active when worktree was created (for merge target)
    pub source_branch: Option<String>,
    /// Original repository root
    pub original_dir: PathBuf,
    /// True if the repository has submodules (requires warning to user)
    pub has_submodules: bool,
}


/// Result of attempting to set up a worktree.
pub enum WorktreeSetupResult {
    /// Successfully created a worktree
    Created(WorktreeInfo),
    /// Not in a git repository, use original directory
    NotAGitRepo,
    /// Git worktree creation failed with error message
    Failed(String),
}

/// Check if a directory is inside a git repository.
pub fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a path is a valid git worktree (not just an existing directory).
///
/// This validates that the directory exists AND is still registered as a git worktree.
/// A directory might exist but not be a valid worktree if `git worktree remove` was run
/// from the original repo without deleting the directory.
pub fn is_valid_worktree(path: &Path) -> bool {
    if !path.exists() || !path.is_dir() {
        return false;
    }

    // Use git rev-parse --is-inside-work-tree to check if it's a valid git worktree
    Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Get git repository information for a directory.
fn get_repo_info(path: &Path) -> Result<GitRepoInfo> {
    // Get repo root
    let output = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Not a git repository");
    }

    let repo_root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());

    // Get current branch
    let branch_output = Command::new("git")
        .current_dir(&repo_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to get current branch")?;

    let branch_str = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();
    let current_branch = if branch_str == "HEAD" {
        None // Detached HEAD
    } else {
        Some(branch_str)
    };

    Ok(GitRepoInfo {
        repo_root,
        current_branch,
    })
}

/// Check if a repository has submodules (checks for non-empty .gitmodules file).
fn has_submodules(repo_root: &Path) -> bool {
    let gitmodules_path = repo_root.join(".gitmodules");
    if !gitmodules_path.exists() {
        return false;
    }
    // Check if file has content (not just exists but is empty)
    std::fs::metadata(&gitmodules_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

/// Create a new worktree for a session.
///
/// If custom_branch is provided, use it; otherwise generate from feature name and session ID.
pub fn create_session_worktree(
    original_dir: &Path,
    session_id: &str,
    feature_name: &str,
    session_dir: &Path,
    custom_branch: Option<&str>,
) -> WorktreeSetupResult {
    // Check if in git repo
    if !is_git_repo(original_dir) {
        return WorktreeSetupResult::NotAGitRepo;
    }

    // Get repo info
    let repo_info = match get_repo_info(original_dir) {
        Ok(info) => info,
        Err(e) => return WorktreeSetupResult::Failed(format!("Failed to get repo info: {}", e)),
    };

    // Check for submodules
    let has_submodules = has_submodules(&repo_info.repo_root);

    // Generate branch name: use custom if provided, otherwise planning-agent/<feature>-<session-short>
    let branch_name = if let Some(custom) = custom_branch {
        custom.to_string()
    } else {
        let short_id = &session_id[..8.min(session_id.len())];
        let safe_feature: String = feature_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();

        // Validate that sanitized feature name isn't empty after sanitization
        let safe_feature = if safe_feature.trim_matches('-').is_empty() {
            "feature".to_string() // Default fallback for empty/all-special-char names
        } else {
            safe_feature
        };

        format!("planning-agent/{}-{}", safe_feature, short_id)
    };

    // Create worktree path
    let worktree_path = session_dir.join("worktree");

    // Create worktree with new branch
    let result = Command::new("git")
        .current_dir(&repo_info.repo_root)
        .args([
            "worktree",
            "add",
            "-b",
            &branch_name,
            &worktree_path.to_string_lossy(),
            "HEAD",
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => WorktreeSetupResult::Created(WorktreeInfo {
            worktree_path,
            branch_name,
            source_branch: repo_info.current_branch,
            original_dir: repo_info.repo_root,
            has_submodules,
        }),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check if branch already exists (resume case)
            if stderr.contains("already exists") {
                // Try with a unique suffix (use millis for better collision avoidance)
                let unique_branch =
                    format!("{}-{}", branch_name, chrono::Utc::now().timestamp_millis());
                let retry = Command::new("git")
                    .current_dir(&repo_info.repo_root)
                    .args([
                        "worktree",
                        "add",
                        "-b",
                        &unique_branch,
                        &worktree_path.to_string_lossy(),
                        "HEAD",
                    ])
                    .output();

                match retry {
                    Ok(o) if o.status.success() => {
                        WorktreeSetupResult::Created(WorktreeInfo {
                            worktree_path,
                            branch_name: unique_branch,
                            source_branch: repo_info.current_branch,
                            original_dir: repo_info.repo_root,
                            has_submodules,
                        })
                    }
                    _ => WorktreeSetupResult::Failed(format!(
                        "Git worktree add failed: {}",
                        stderr
                    )),
                }
            } else {
                WorktreeSetupResult::Failed(format!("Git worktree add failed: {}", stderr))
            }
        }
        Err(e) => WorktreeSetupResult::Failed(format!("Failed to run git: {}", e)),
    }
}

/// Remove a worktree and optionally its branch.
pub fn remove_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    branch_name: Option<&str>,
) -> Result<()> {
    // Remove the worktree
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "worktree",
            "remove",
            "--force",
            &worktree_path.to_string_lossy(),
        ])
        .output()
        .context("Failed to run git worktree remove")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to remove worktree: {}", stderr);
    }

    // Also remove the branch to avoid orphaned branches
    if let Some(branch) = branch_name {
        let _ = Command::new("git")
            .current_dir(repo_root)
            .args(["branch", "-d", branch])
            .output();
        // Ignore branch deletion errors - branch may have been merged or deleted already
    }

    Ok(())
}

/// Generate merge instructions for the user.
pub fn generate_merge_instructions(info: &WorktreeInfo) -> String {
    let target = info.source_branch.as_deref().unwrap_or("main");

    format!(
        r#"
=== GIT MERGE INSTRUCTIONS ===

Your changes are on branch: {}
Target branch for merge: {}

To merge your changes:
  cd {}
  git checkout {}
  git merge {}

To view changes before merging:
  git diff {}..{}

To create a pull request instead:
  git push -u origin {}
  # Then create PR from {} to {}

To remove the worktree after merging:
  git worktree remove {}
  git branch -d {}
"#,
        info.branch_name,
        target,
        info.original_dir.display(),
        target,
        info.branch_name,
        target,
        info.branch_name,
        info.branch_name,
        info.branch_name,
        target,
        info.worktree_path.display(),
        info.branch_name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_git_repo_non_git() {
        let dir = tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
    }

    #[test]
    fn test_is_git_repo_with_git() {
        let dir = tempdir().unwrap();

        // Initialize a git repo
        let output = Command::new("git")
            .current_dir(dir.path())
            .args(["init"])
            .output();

        if output.is_ok() && output.unwrap().status.success() {
            assert!(is_git_repo(dir.path()));
        }
    }

    #[test]
    fn test_is_valid_worktree_non_existent() {
        let path = PathBuf::from("/non/existent/path");
        assert!(!is_valid_worktree(&path));
    }

    #[test]
    fn test_is_valid_worktree_non_git_dir() {
        let dir = tempdir().unwrap();
        assert!(!is_valid_worktree(dir.path()));
    }

    #[test]
    fn test_create_session_worktree_not_git_repo() {
        let dir = tempdir().unwrap();
        let session_dir = tempdir().unwrap();

        let result = create_session_worktree(
            dir.path(),
            "test-session-id",
            "test-feature",
            session_dir.path(),
            None,
        );

        matches!(result, WorktreeSetupResult::NotAGitRepo);
    }

    #[test]
    fn test_branch_name_sanitization() {
        // Test that feature names with special characters get sanitized
        let sanitized: String = "test/feature@with!special#chars"
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        assert_eq!(sanitized, "test-feature-with-special-chars");
    }

    #[test]
    fn test_branch_name_empty_after_sanitization() {
        // Test fallback for names that become empty after sanitization
        let sanitized: String = "!!@@##"
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        let safe = if sanitized.trim_matches('-').is_empty() {
            "feature".to_string()
        } else {
            sanitized
        };
        assert_eq!(safe, "feature");
    }

    #[test]
    fn test_has_submodules_no_file() {
        let dir = tempdir().unwrap();
        assert!(!has_submodules(dir.path()));
    }

    #[test]
    fn test_has_submodules_empty_file() {
        let dir = tempdir().unwrap();
        let gitmodules_path = dir.path().join(".gitmodules");
        std::fs::write(&gitmodules_path, "").unwrap();
        assert!(!has_submodules(dir.path()));
    }

    #[test]
    fn test_has_submodules_with_content() {
        let dir = tempdir().unwrap();
        let gitmodules_path = dir.path().join(".gitmodules");
        std::fs::write(&gitmodules_path, "[submodule \"test\"]").unwrap();
        assert!(has_submodules(dir.path()));
    }

    #[test]
    fn test_generate_merge_instructions() {
        let info = WorktreeInfo {
            worktree_path: PathBuf::from("/tmp/worktree"),
            branch_name: "planning-agent/my-feature-abc123".to_string(),
            source_branch: Some("main".to_string()),
            original_dir: PathBuf::from("/home/user/repo"),
            has_submodules: false,
        };

        let instructions = generate_merge_instructions(&info);

        assert!(instructions.contains("planning-agent/my-feature-abc123"));
        assert!(instructions.contains("main"));
        assert!(instructions.contains("git checkout main"));
        assert!(instructions.contains("git merge planning-agent/my-feature-abc123"));
    }

    #[test]
    fn test_generate_merge_instructions_no_source_branch() {
        let info = WorktreeInfo {
            worktree_path: PathBuf::from("/tmp/worktree"),
            branch_name: "planning-agent/feature-abc123".to_string(),
            source_branch: None, // Detached HEAD case
            original_dir: PathBuf::from("/home/user/repo"),
            has_submodules: false,
        };

        let instructions = generate_merge_instructions(&info);

        // Should default to "main" when source_branch is None
        assert!(instructions.contains("Target branch for merge: main"));
    }
}
