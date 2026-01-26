//! Per-session context for working directory and configuration.
//!
//! This module provides `SessionContext` which tracks session-specific paths
//! and configuration, enabling cross-directory session resume without spawning
//! new terminals.

use crate::config::WorkflowConfig;
use crate::domain::types::WorktreeState;
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
#[path = "tests/context_tests.rs"]
mod tests;
