//! Centralized home-based storage paths for all planning-agent persistence.
//!
//! This module provides helpers for unified storage under `~/.planning-agent/`:
//! - `sessions/` - Session data including event logs, snapshots, plans, and feedback
//! - `logs/<wd-hash>/` - Workflow and agent logs (qualified by working directory)
//! - `logs/debug.log` - Debug log
//! - `update-installed` - Update marker
//!
//! # Configuration
//!
//! The home directory can be configured via:
//! 1. `PLANNING_AGENT_HOME` environment variable (for production/container use)
//! 2. `set_home_for_test()` (for test isolation - test-only API)
//!
//! All path functions in this module use `planning_agent_home_dir()` as the
//! single source of truth.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The name of the planning agent directory.
const PLANNING_AGENT_DIR: &str = ".planning-agent";

// Thread-local override for testing. Takes precedence over env var.
#[cfg(test)]
std::thread_local! {
    static TEST_HOME_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Set the planning agent home directory for the current test.
/// This is the ONLY way tests should override the home directory.
/// Returns a guard that restores the previous value when dropped.
#[cfg(test)]
pub fn set_home_for_test(path: PathBuf) -> TestHomeGuard {
    let previous = TEST_HOME_OVERRIDE.with(|cell| cell.borrow_mut().replace(path));
    TestHomeGuard { previous }
}

/// Guard that restores the previous home override when dropped.
#[cfg(test)]
pub struct TestHomeGuard {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        TEST_HOME_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
    }
}

/// Returns the home-based planning agent directory: `~/.planning-agent/`
///
/// Resolution order:
/// 1. Test override (if set via `set_home_for_test()`)
/// 2. `PLANNING_AGENT_HOME` environment variable
/// 3. Default: `~/.planning-agent/`
///
/// Creates the directory if it doesn't exist.
///
/// # Errors
///
/// Returns an error if:
/// - Home directory cannot be determined (when env var not set)
/// - Directory creation fails
pub fn planning_agent_home_dir() -> Result<PathBuf> {
    // Check test override first (only in test builds)
    #[cfg(test)]
    {
        if let Some(path) = TEST_HOME_OVERRIDE.with(|cell| cell.borrow().clone()) {
            fs::create_dir_all(&path).with_context(|| {
                format!(
                    "Failed to create test planning directory: {}",
                    path.display()
                )
            })?;
            return Ok(path);
        }
    }

    let planning_dir = if let Ok(custom_home) = std::env::var("PLANNING_AGENT_HOME") {
        PathBuf::from(custom_home)
    } else {
        let home =
            dirs::home_dir().context("Could not determine home directory for plan storage")?;
        home.join(PLANNING_AGENT_DIR)
    };
    fs::create_dir_all(&planning_dir).with_context(|| {
        format!(
            "Failed to create planning directory: {}",
            planning_dir.display()
        )
    })?;
    Ok(planning_dir)
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

/// Returns the startup log path: `~/.planning-agent/logs/startup.log`
///
/// This is used for early logging before a session is created.
/// Entries are merged into the session log when `SessionLogger::new` is called.
pub fn startup_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("startup.log"))
}

/// Returns the update marker path: `~/.planning-agent/update-installed`
pub fn update_marker_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("update-installed"))
}

/// Returns the version cache path: `~/.planning-agent/version-cache.json`
pub fn version_cache_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("version-cache.json"))
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
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ============================================================================
// Session-Centric Paths (New Consolidated Structure)
// ============================================================================

/// Returns the session directory: `~/.planning-agent/sessions/<session-id>/`
///
/// Creates the directory if it doesn't exist.
pub fn session_dir(session_id: &str) -> Result<PathBuf> {
    let dir = sessions_dir()?.join(session_id);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create session directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the session state file: `~/.planning-agent/sessions/<session-id>/state.json`
pub fn session_state_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("state.json"))
}

/// Returns the session plan file: `~/.planning-agent/sessions/<session-id>/plan.md`
pub fn session_plan_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("plan.md"))
}

/// Returns the session feedback file: `~/.planning-agent/sessions/<session-id>/feedback_<round>.md`
pub fn session_feedback_path(session_id: &str, round: u32) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join(format!("feedback_{}.md", round)))
}

/// Returns the session resume snapshot file: `~/.planning-agent/sessions/<session-id>/session.json`
///
/// This file contains the TUI session state for resume functionality.
pub fn session_snapshot_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("session.json"))
}

/// Returns the session event log file: `~/.planning-agent/sessions/<session-id>/events.jsonl`
///
/// This file contains the CQRS event log in JSONL format.
pub fn session_event_log_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("events.jsonl"))
}

/// Returns the session aggregate snapshot file: `~/.planning-agent/sessions/<session-id>/snapshot.json`
///
/// This file contains the CQRS aggregate state snapshot for faster event replay.
pub fn session_aggregate_snapshot_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("snapshot.json"))
}

/// Returns the session logs directory: `~/.planning-agent/sessions/<session-id>/logs/`
///
/// Creates the directory if it doesn't exist.
pub fn session_logs_dir(session_id: &str) -> Result<PathBuf> {
    let dir = session_dir(session_id)?.join("logs");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create session logs directory: {}", dir.display()))?;
    Ok(dir)
}

// ============================================================================
// Implementation Phase Paths
// ============================================================================

/// Returns the implementation log path for a given iteration.
/// Format: `~/.planning-agent/sessions/<session-id>/implementation_<iteration>.log`
pub fn session_implementation_log_path(session_id: &str, iteration: u32) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join(format!("implementation_{}.log", iteration)))
}

/// Returns the implementation review report path for a given iteration.
/// Format: `~/.planning-agent/sessions/<session-id>/implementation_review_<iteration>.md`
pub fn session_implementation_review_path(session_id: &str, iteration: u32) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join(format!("implementation_review_{}.md", iteration)))
}

/// Returns the session info metadata file: `~/.planning-agent/sessions/<session-id>/session_info.json`
pub fn session_info_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("session_info.json"))
}

// ============================================================================
// Session Daemon Paths
// ============================================================================

/// Returns the session daemon PID file path: `~/.planning-agent/sessiond.pid`
pub fn sessiond_pid_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.pid"))
}

/// Returns the session daemon lock file path: `~/.planning-agent/sessiond.lock`
pub fn sessiond_lock_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.lock"))
}

/// Returns the session daemon build SHA file path: `~/.planning-agent/sessiond.sha`
///
/// Used for version detection when daemon is unresponsive.
pub fn sessiond_build_sha_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.sha"))
}

/// Returns the session daemon port file path: `~/.planning-agent/sessiond.port`
///
/// Contains JSON with port number, subscriber port, and authentication token.
pub fn sessiond_port_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.port"))
}

/// Returns the session daemon registry file path: `~/.planning-agent/sessiond.registry.json`
///
/// Used for faster recovery after daemon restart.
pub fn sessiond_registry_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.registry.json"))
}

/// Returns the diagnostics directory for a working directory: `~/.planning-agent/diagnostics/<wd-hash>/`
///
/// Creates the directory if it doesn't exist.
pub fn diagnostics_dir(working_dir: &Path) -> Result<PathBuf> {
    let hash = working_dir_hash(working_dir);
    let dir = planning_agent_home_dir()?.join("diagnostics").join(&hash);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create diagnostics directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the path for a review diagnostics bundle.
///
/// Format: `~/.planning-agent/diagnostics/<wd-hash>/review-<agent>-<timestamp>-<suffix>.zip`
pub fn review_bundle_path(
    working_dir: &Path,
    agent_name: &str,
    timestamp: &str,
    suffix: &str,
) -> Result<PathBuf> {
    let filename = format!("review-{}-{}-{}.zip", agent_name, timestamp, suffix);
    Ok(diagnostics_dir(working_dir)?.join(filename))
}

/// Lightweight session info for fast listing without loading full snapshots.
///
/// This struct is stored in `session_info.json` within each session directory
/// and updated on each state save for efficient session listing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionInfo {
    /// The workflow session ID
    pub session_id: String,
    /// Human-readable feature name
    pub feature_name: String,
    /// Brief objective description
    pub objective: String,
    /// Working directory where the workflow was started
    pub working_dir: PathBuf,
    /// Session creation timestamp (RFC3339)
    pub created_at: String,
    /// Last update timestamp (RFC3339)
    pub updated_at: String,
    /// Current workflow phase
    pub phase: String,
    /// Current iteration number
    pub iteration: u32,
}

impl SessionInfo {
    /// Creates a new SessionInfo with the current timestamp.
    pub fn new(
        session_id: &str,
        feature_name: &str,
        objective: &str,
        working_dir: &Path,
        phase: &str,
        iteration: u32,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            session_id: session_id.to_string(),
            feature_name: feature_name.to_string(),
            objective: objective.to_string(),
            working_dir: working_dir.to_path_buf(),
            created_at: now.clone(),
            updated_at: now,
            phase: phase.to_string(),
            iteration,
        }
    }

    /// Saves the session info to the session_info.json file.
    pub fn save(&self, session_id: &str) -> Result<()> {
        let path = session_info_path(session_id)?;
        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize session info")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write session info: {}", path.display()))?;
        Ok(())
    }

    /// Loads session info from the session_info.json file.
    #[cfg(test)]
    pub fn load(session_id: &str) -> Result<Self> {
        let path = session_info_path(session_id)?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session info: {}", path.display()))?;
        serde_json::from_str(&content).with_context(|| "Failed to parse session info")
    }
}

#[cfg(test)]
#[path = "tests/planning_paths_tests.rs"]
mod tests;
