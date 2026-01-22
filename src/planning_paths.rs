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
    let home = dirs::home_dir().context("Could not determine home directory for plan storage")?;
    let planning_dir = home.join(PLANNING_AGENT_DIR);
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

/// Returns the codex status log path: `~/.planning-agent/logs/codex-status.log`
pub fn codex_status_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("codex-status.log"))
}

/// Returns the Claude usage debug log path: `~/.planning-agent/logs/claude-usage.log`
pub fn claude_usage_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("claude-usage.log"))
}

/// Returns the Gemini usage debug log path: `~/.planning-agent/logs/gemini-usage.log`
pub fn gemini_usage_log_path() -> Result<PathBuf> {
    let logs = planning_agent_home_dir()?.join("logs");
    fs::create_dir_all(&logs)
        .with_context(|| format!("Failed to create logs directory: {}", logs.display()))?;
    Ok(logs.join("gemini-usage.log"))
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

/// Returns the session snapshot file: `~/.planning-agent/sessions/<session-id>/session.json`
pub fn session_snapshot_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("session.json"))
}

/// Returns the session logs directory: `~/.planning-agent/sessions/<session-id>/logs/`
///
/// Creates the directory if it doesn't exist.
#[allow(dead_code)]
pub fn session_logs_dir(session_id: &str) -> Result<PathBuf> {
    let dir = session_dir(session_id)?.join("logs");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create session logs directory: {}", dir.display()))?;
    Ok(dir)
}

/// Returns the session main log file: `~/.planning-agent/sessions/<session-id>/logs/session.log`
#[allow(dead_code)]
pub fn session_log_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_logs_dir(session_id)?.join("session.log"))
}

/// Returns the session agent stream log: `~/.planning-agent/sessions/<session-id>/logs/agent-stream.log`
#[allow(dead_code)]
pub fn session_agent_log_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_logs_dir(session_id)?.join("agent-stream.log"))
}

/// Returns the session workflow log: `~/.planning-agent/sessions/<session-id>/logs/workflow.log`
#[allow(dead_code)]
pub fn session_workflow_log_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_logs_dir(session_id)?.join("workflow.log"))
}

// ============================================================================
// Implementation Phase Paths
// ============================================================================

/// Returns the implementation log path for a given iteration.
/// Format: `~/.planning-agent/sessions/<session-id>/implementation_<iteration>.log`
#[allow(dead_code)]
pub fn session_implementation_log_path(session_id: &str, iteration: u32) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join(format!("implementation_{}.log", iteration)))
}

/// Returns the implementation review report path for a given iteration.
/// Format: `~/.planning-agent/sessions/<session-id>/implementation_review_<iteration>.md`
#[allow(dead_code)]
pub fn session_implementation_review_path(session_id: &str, iteration: u32) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join(format!("implementation_review_{}.md", iteration)))
}

/// Returns the implementation change fingerprint file path.
/// Format: `~/.planning-agent/sessions/<session-id>/implementation_fingerprint.json`
///
/// This file stores a hash of repository changes to detect if implementation
/// has been modified between orchestrator runs.
#[allow(dead_code)]
pub fn session_implementation_fingerprint_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("implementation_fingerprint.json"))
}

/// Returns the session diagnostics directory: `~/.planning-agent/sessions/<session-id>/diagnostics/`
///
/// Creates the directory if it doesn't exist.
#[allow(dead_code)]
pub fn session_diagnostics_dir(session_id: &str) -> Result<PathBuf> {
    let dir = session_dir(session_id)?.join("diagnostics");
    fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create session diagnostics directory: {}",
            dir.display()
        )
    })?;
    Ok(dir)
}

/// Returns the session info metadata file: `~/.planning-agent/sessions/<session-id>/session_info.json`
pub fn session_info_path(session_id: &str) -> Result<PathBuf> {
    Ok(session_dir(session_id)?.join("session_info.json"))
}

// ============================================================================
// Session Daemon Paths
// ============================================================================

/// Returns the session daemon socket path: `~/.planning-agent/sessiond.sock` (Unix only)
#[cfg(unix)]
pub fn sessiond_socket_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.sock"))
}

/// Returns the session daemon PID file path: `~/.planning-agent/sessiond.pid`
pub fn sessiond_pid_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.pid"))
}

/// Returns the session daemon lock file path: `~/.planning-agent/sessiond.lock`
#[allow(dead_code)]
pub fn sessiond_lock_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.lock"))
}

/// Returns the session daemon build SHA file path: `~/.planning-agent/sessiond.sha`
///
/// Used for version detection when daemon is unresponsive.
pub fn sessiond_build_sha_path() -> Result<PathBuf> {
    Ok(planning_agent_home_dir()?.join("sessiond.sha"))
}

/// Returns the session daemon port file path: `~/.planning-agent/sessiond.port` (Windows)
///
/// Contains JSON with port number and authentication token.
#[cfg(windows)]
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

/// Metadata about a plan folder for listing purposes.
#[derive(Debug, Clone)]
pub struct PlanInfo {
    /// Full path to the plan folder
    pub path: PathBuf,
    /// Feature name extracted from the folder name
    pub feature_name: String,
    /// Timestamp string from the folder name (YYYYMMDD-HHMMSS)
    pub timestamp: String,
    /// Full folder name (timestamp_feature-name)
    pub folder_name: String,
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

    /// Updates the session info with a new phase and iteration.
    #[allow(dead_code)]
    pub fn update(&mut self, phase: &str, iteration: u32) {
        self.phase = phase.to_string();
        self.iteration = iteration;
        self.updated_at = chrono::Utc::now().to_rfc3339();
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
    #[allow(dead_code)]
    pub fn load(session_id: &str) -> Result<Self> {
        let path = session_info_path(session_id)?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session info: {}", path.display()))?;
        serde_json::from_str(&content).with_context(|| "Failed to parse session info")
    }
}

/// Lists all plan folders in session directories.
///
/// Returns a vector of PlanInfo, sorted by timestamp descending (most recent first).
/// Scans `~/.planning-agent/sessions/` directories.
pub fn list_plans() -> Result<Vec<PlanInfo>> {
    let mut plans = Vec::new();

    // Scan session directories: ~/.planning-agent/sessions/<session-id>/plan.md
    let sessions_directory = sessions_dir()?;
    if sessions_directory.exists() {
        for entry in fs::read_dir(&sessions_directory)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            // Check if plan.md exists in this session folder
            let plan_file = path.join("plan.md");
            if !plan_file.exists() {
                continue;
            }

            let session_id = entry.file_name().to_string_lossy().to_string();

            // Try to read session_info.json for metadata
            let info_path = path.join("session_info.json");
            let (feature_name, timestamp) = if info_path.exists() {
                if let Ok(content) = fs::read_to_string(&info_path) {
                    if let Ok(info) = serde_json::from_str::<SessionInfo>(&content) {
                        // Convert RFC3339 timestamp to YYYYMMDD-HHMMSS format
                        let ts = convert_rfc3339_to_timestamp(&info.created_at)
                            .unwrap_or_else(|| info.created_at.clone());
                        (info.feature_name, ts)
                    } else {
                        (session_id.clone(), String::new())
                    }
                } else {
                    (session_id.clone(), String::new())
                }
            } else {
                // Fallback: use session_id as feature name
                (session_id.clone(), String::new())
            };

            plans.push(PlanInfo {
                path,
                feature_name,
                timestamp,
                folder_name: session_id,
            });
        }
    }

    // Sort by timestamp descending (most recent first)
    plans.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(plans)
}

/// Converts an RFC3339 timestamp to YYYYMMDD-HHMMSS format.
pub fn convert_rfc3339_to_timestamp(rfc3339: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|dt| dt.format("%Y%m%d-%H%M%S").to_string())
}

/// Finds a plan folder by partial match on feature name or folder name.
///
/// Returns the most recent matching plan if multiple matches are found.
pub fn find_plan(pattern: &str) -> Result<Option<PlanInfo>> {
    let plans = list_plans()?;

    let pattern_lower = pattern.to_lowercase();

    // First try exact match on folder name
    for plan in &plans {
        if plan.folder_name.to_lowercase() == pattern_lower {
            return Ok(Some(plan.clone()));
        }
    }

    // Then try exact match on feature name
    for plan in &plans {
        if plan.feature_name.to_lowercase() == pattern_lower {
            return Ok(Some(plan.clone()));
        }
    }

    // Then try partial match on feature name or folder name
    for plan in &plans {
        if plan.feature_name.to_lowercase().contains(&pattern_lower)
            || plan.folder_name.to_lowercase().contains(&pattern_lower)
        {
            return Ok(Some(plan.clone()));
        }
    }

    Ok(None)
}

/// Returns the most recently created plan folder.
pub fn latest_plan() -> Result<Option<PlanInfo>> {
    let plans = list_plans()?;
    Ok(plans.into_iter().next())
}

#[cfg(test)]
#[path = "planning_paths_tests.rs"]
mod tests;
