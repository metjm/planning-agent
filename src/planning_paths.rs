//! Centralized home-based storage paths for all planning-agent persistence.
//!
//! This module provides helpers for unified storage under `~/.planning-agent/`:
//! - `plans/` - Plan and feedback files
//! - `sessions/` - Session snapshots
//! - `state/<wd-hash>/` - Workflow state files (qualified by working directory)
//! - `logs/<wd-hash>/` - Workflow and agent logs (qualified by working directory)
//! - `logs/debug.log` - Debug log
//! - `update-installed` - Update marker

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The name of the planning agent directory.
const PLANNING_AGENT_DIR: &str = ".planning-agent";

/// Returns the home-based planning agent directory: `~/.planning-agent/`
///
/// Creates the directory if it doesn't exist.
///
/// # Errors
///
/// Returns an error if:
/// - Home directory cannot be determined
/// - Directory creation fails
pub fn planning_agent_home_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .context("Could not determine home directory for plan storage")?;
    let planning_dir = home.join(PLANNING_AGENT_DIR);
    fs::create_dir_all(&planning_dir)
        .with_context(|| format!("Failed to create planning directory: {}", planning_dir.display()))?;
    Ok(planning_dir)
}

/// Returns the plans directory: `~/.planning-agent/plans/`
///
/// Creates the directory if it doesn't exist.
pub fn plans_dir() -> Result<PathBuf> {
    let dir = planning_agent_home_dir()?.join("plans");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create plans directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the sessions directory: `~/.planning-agent/sessions/`
///
/// Creates the directory if it doesn't exist.
pub fn sessions_dir() -> Result<PathBuf> {
    let dir = planning_agent_home_dir()?.join("sessions");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create sessions directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the state directory for a working directory: `~/.planning-agent/state/<wd-hash>/`
///
/// Creates the directory if it doesn't exist.
pub fn state_dir(working_dir: &Path) -> Result<PathBuf> {
    let hash = working_dir_hash(working_dir);
    let dir = planning_agent_home_dir()?.join("state").join(&hash);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create state directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the full path for a state file: `~/.planning-agent/state/<wd-hash>/<feature>.json`
pub fn state_path(working_dir: &Path, feature_name: &str) -> Result<PathBuf> {
    Ok(state_dir(working_dir)?.join(format!("{}.json", feature_name)))
}

/// Returns the logs directory for a working directory: `~/.planning-agent/logs/<wd-hash>/`
///
/// Creates the directory if it doesn't exist.
pub fn logs_dir(working_dir: &Path) -> Result<PathBuf> {
    let hash = working_dir_hash(working_dir);
    let dir = planning_agent_home_dir()?.join("logs").join(&hash);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create logs directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the debug log path: `~/.planning-agent/logs/debug.log`
pub fn debug_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("debug.log"))
}

/// Returns the update marker path: `~/.planning-agent/update-installed`
pub fn update_marker_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("update-installed"))
}

/// Returns the codex status log path: `~/.planning-agent/logs/codex-status.log`
pub fn codex_status_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("codex-status.log"))
}

/// Computes a working directory hash (SHA256 truncated to 12 hex characters).
///
/// Attempts to canonicalize the path first for consistency across symlinks.
/// Falls back to hashing the raw path bytes if canonicalization fails.
pub fn working_dir_hash(path: &Path) -> String {
    // Try to canonicalize for consistent results across symlinks
    let bytes = match fs::canonicalize(path) {
        Ok(canonical) => canonical.to_string_lossy().into_owned().into_bytes(),
        Err(_) => {
            // Fallback: hash raw path bytes (handles deleted directories or non-UTF8 paths)
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                path.as_os_str().as_bytes().to_vec()
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, use lossy conversion
                path.to_string_lossy().into_owned().into_bytes()
            }
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();

    // Take first 6 bytes (12 hex characters)
    hex_encode(&result[..6])
}

/// Encodes bytes as lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Returns the workflow log path: `~/.planning-agent/logs/<wd-hash>/workflow-<run>.log`
pub fn workflow_log_path(working_dir: &Path, run_id: &str) -> Result<PathBuf> {
    Ok(logs_dir(working_dir)?.join(format!("workflow-{}.log", run_id)))
}

/// Returns the agent stream log path: `~/.planning-agent/logs/<wd-hash>/agent-stream-<run>.log`
pub fn agent_stream_log_path(working_dir: &Path, run_id: &str) -> Result<PathBuf> {
    Ok(logs_dir(working_dir)?.join(format!("agent-stream-{}.log", run_id)))
}

/// Returns the snapshot file path for a session: `~/.planning-agent/sessions/<session-id>.json`
pub fn snapshot_path(session_id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{}.json", session_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::tempdir;

    #[test]
    fn test_working_dir_hash_consistency() {
        let dir = tempdir().unwrap();
        let path = dir.path();

        let hash1 = working_dir_hash(path);
        let hash2 = working_dir_hash(path);

        assert_eq!(hash1, hash2, "Hash should be consistent across calls");
        assert_eq!(hash1.len(), 12, "Hash should be 12 hex characters");
    }

    #[test]
    fn test_working_dir_hash_different_paths() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let hash1 = working_dir_hash(dir1.path());
        let hash2 = working_dir_hash(dir2.path());

        assert_ne!(hash1, hash2, "Different paths should produce different hashes");
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x10]), "00ff10");
        assert_eq!(hex_encode(&[0xab, 0xcd, 0xef]), "abcdef");
    }

    #[test]
    fn test_planning_agent_home_dir() {
        // Skip if HOME is not set
        if env::var("HOME").is_err() {
            return;
        }

        let result = planning_agent_home_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with(".planning-agent"));
    }

    #[test]
    fn test_plans_dir() {
        if env::var("HOME").is_err() {
            return;
        }

        let result = plans_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with("plans"));
    }

    #[test]
    fn test_sessions_dir() {
        if env::var("HOME").is_err() {
            return;
        }

        let result = sessions_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with("sessions"));
    }

    #[test]
    fn test_state_path() {
        if env::var("HOME").is_err() {
            return;
        }

        let dir = tempdir().unwrap();
        let result = state_path(dir.path(), "my-feature");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with("my-feature.json"));
        assert!(path.to_string_lossy().contains("state"));
    }

    #[test]
    fn test_debug_log_path() {
        if env::var("HOME").is_err() {
            return;
        }

        let result = debug_log_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with("debug.log"));
        assert!(path.to_string_lossy().contains("logs"));
    }

    #[test]
    fn test_update_marker_path() {
        if env::var("HOME").is_err() {
            return;
        }

        let result = update_marker_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.ends_with("update-installed"));
    }
}
