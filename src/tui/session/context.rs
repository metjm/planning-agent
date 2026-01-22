//! Per-session context for working directory and configuration.
//!
//! This module provides `SessionContext` which tracks session-specific paths
//! and configuration, enabling cross-directory session resume without spawning
//! new terminals.

use crate::config::WorkflowConfig;
use crate::state::WorktreeState;
use std::path::{Path, PathBuf};

/// Per-session context tracking working directory, paths, and configuration.
///
/// This context enables sessions from different directories to coexist in
/// the same TUI process by tracking directory-specific state per session.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Base working directory (used for state/log paths, persisted in snapshots).
    /// This is the directory passed to `planning` CLI or stored in session snapshot.
    pub base_working_dir: PathBuf,

    /// Effective working directory (worktree path if created, otherwise base).
    /// This is where agents execute and where file index is scoped.
    pub effective_working_dir: PathBuf,

    /// Path to state file for this session.
    pub state_path: PathBuf,

    /// Workflow configuration for this session.
    pub workflow_config: WorkflowConfig,
}

impl SessionContext {
    /// Creates a new SessionContext with the given parameters.
    ///
    /// If `effective_working_dir` is not provided, it defaults to `base_working_dir`.
    pub fn new(
        base_working_dir: PathBuf,
        effective_working_dir: Option<PathBuf>,
        state_path: PathBuf,
        workflow_config: WorkflowConfig,
    ) -> Self {
        let effective = effective_working_dir.unwrap_or_else(|| base_working_dir.clone());
        Self {
            base_working_dir,
            effective_working_dir: effective,
            state_path,
            workflow_config,
        }
    }

    /// Creates a SessionContext from a snapshot's data.
    ///
    /// Computes `effective_working_dir` from worktree_info if present.
    /// Falls back to `base_working_dir` if worktree path doesn't exist.
    pub fn from_snapshot(
        base_working_dir: PathBuf,
        state_path: PathBuf,
        worktree_info: Option<&WorktreeState>,
        workflow_config: WorkflowConfig,
    ) -> Self {
        let effective_working_dir = compute_effective_working_dir(&base_working_dir, worktree_info);

        Self {
            base_working_dir,
            effective_working_dir,
            state_path,
            workflow_config,
        }
    }
}

/// Computes the effective working directory from base and worktree info.
///
/// If worktree_info is present and the worktree path exists, returns the worktree path.
/// Otherwise returns the base_working_dir.
///
/// This function logs a warning if a worktree path is configured but no longer exists.
pub fn compute_effective_working_dir(
    base_working_dir: &Path,
    worktree_info: Option<&WorktreeState>,
) -> PathBuf {
    if let Some(wt) = worktree_info {
        if wt.worktree_path.exists() {
            return wt.worktree_path.clone();
        }
        // Worktree path doesn't exist - log warning and fall back
        eprintln!(
            "[planning] Warning: Worktree path no longer exists: {}",
            wt.worktree_path.display()
        );
        eprintln!(
            "[planning] Falling back to base directory: {}",
            base_working_dir.display()
        );
    }
    base_working_dir.to_path_buf()
}

/// Validates that a working directory exists.
///
/// Returns an error message if the directory doesn't exist or isn't accessible.
pub fn validate_working_dir(working_dir: &Path) -> Result<(), String> {
    if !working_dir.exists() {
        return Err(format!(
            "Working directory no longer exists: {}",
            working_dir.display()
        ));
    }
    if !working_dir.is_dir() {
        return Err(format!(
            "Working directory is not a directory: {}",
            working_dir.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_context_new_defaults_effective_to_base() {
        let config = WorkflowConfig::default_config();
        let ctx = SessionContext::new(
            PathBuf::from("/base/dir"),
            None,
            PathBuf::from("/state/path.json"),
            config,
        );

        assert_eq!(ctx.base_working_dir, PathBuf::from("/base/dir"));
        assert_eq!(ctx.effective_working_dir, PathBuf::from("/base/dir"));
    }

    #[test]
    fn test_context_new_with_effective() {
        let config = WorkflowConfig::default_config();
        let ctx = SessionContext::new(
            PathBuf::from("/base/dir"),
            Some(PathBuf::from("/worktree/dir")),
            PathBuf::from("/state/path.json"),
            config,
        );

        assert_eq!(ctx.base_working_dir, PathBuf::from("/base/dir"));
        assert_eq!(ctx.effective_working_dir, PathBuf::from("/worktree/dir"));
    }

    #[test]
    fn test_compute_effective_without_worktree() {
        let base = PathBuf::from("/base/dir");
        let result = compute_effective_working_dir(&base, None);
        assert_eq!(result, base);
    }

    #[test]
    fn test_compute_effective_with_nonexistent_worktree() {
        let base = PathBuf::from("/base/dir");
        let wt = WorktreeState {
            worktree_path: PathBuf::from("/nonexistent/worktree"),
            branch_name: "feature".to_string(),
            source_branch: Some("main".to_string()),
            original_dir: base.clone(),
        };
        let result = compute_effective_working_dir(&base, Some(&wt));
        // Should fall back to base since worktree doesn't exist
        assert_eq!(result, base);
    }

    #[test]
    fn test_validate_working_dir_nonexistent() {
        let result = validate_working_dir(Path::new("/definitely/does/not/exist/12345"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no longer exists"));
    }
}
