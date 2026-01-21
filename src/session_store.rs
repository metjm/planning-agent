//! Session snapshot persistence for stop/resume functionality.
//!
//! This module provides the ability to persist workflow sessions to disk,
//! allowing users to stop a running workflow and resume it later.
//!
//! ## Design Decisions
//!
//! - **Per-workflow snapshots**: Each snapshot contains exactly one workflow session,
//!   NOT the entire TabManager state. This provides clear resume semantics.
//! - **Snapshot location**: `~/.planning-agent/sessions/<workflow_session_id>.json`
//! - **Versioned format**: Snapshots include a version field for future migrations.

use crate::cli_usage::AccountUsage;
use crate::planning_paths;
use crate::state::State;
use crate::tui::session::model::{
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, PasteBlock, ReviewRound,
    RunTab, SessionStatus, TodoItem,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Current snapshot format version.
/// Increment this when making breaking changes to the snapshot format.
///
/// Version history:
/// - v1: Initial version with session_used/weekly_used in ProviderUsage
/// - v2: UsageWindow with reset timestamps (session/weekly fields)
/// - v3: Removed embedded implementation terminal (InputMode/FocusedPanel have Unknown variants)
/// - v4: Run-tab entries include tool timeline entries (no migration support)
pub const SNAPSHOT_VERSION: u32 = 4;

/// A persistable snapshot of a workflow session.
///
/// Contains both workflow state and UI state, allowing full session restoration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Snapshot format version for migration compatibility
    pub version: u32,
    /// Timestamp when this snapshot was created (RFC3339 format)
    pub saved_at: String,
    /// Working directory where the workflow was running
    pub working_dir: PathBuf,
    /// The workflow session ID (from State.workflow_session_id)
    pub workflow_session_id: String,
    /// Path to the state file for this workflow
    pub state_path: PathBuf,
    /// The workflow state at time of snapshot
    pub workflow_state: State,
    /// UI state that can be restored
    pub ui_state: SessionUiState,
    /// Total elapsed time before this resume (milliseconds).
    /// Accumulated across multiple stop/resume cycles.
    pub total_elapsed_before_resume_ms: u64,
}

/// Serializable subset of Session that captures UI state.
///
/// Excludes non-serializable fields like `JoinHandle`, `Instant`, and channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUiState {
    pub id: usize,
    pub name: String,
    pub status: SessionStatus,

    // Output state
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub output_follow_mode: bool,
    pub streaming_lines: Vec<String>,
    pub streaming_scroll_position: usize,
    pub streaming_follow_mode: bool,
    pub focused_panel: FocusedPanel,

    // Cost and metrics
    pub total_cost: f64,
    pub bytes_received: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub tool_call_count: usize,
    pub bytes_per_second: f64,
    pub turn_count: u32,
    pub model_name: Option<String>,
    pub last_stop_reason: Option<String>,
    pub tool_error_count: usize,
    pub total_tool_duration_ms: u64,
    pub completed_tool_count: usize,

    // Approval state
    pub approval_mode: ApprovalMode,
    pub approval_context: ApprovalContext,
    pub plan_summary: String,
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,
    pub feedback_scroll: usize,
    pub feedback_target: FeedbackTarget,

    // Input state
    pub input_mode: InputMode,
    pub tab_input: String,
    pub tab_input_cursor: usize,
    pub tab_input_scroll: usize,
    pub last_key_was_backslash: bool,
    pub tab_input_pastes: Vec<PasteBlock>,
    pub feedback_pastes: Vec<PasteBlock>,

    // Error state
    pub error_state: Option<String>,
    pub error_scroll: usize,

    // Run tabs and todos
    pub run_tabs: Vec<RunTab>,
    pub active_run_tab: usize,
    pub chat_follow_mode: bool,
    pub todos: HashMap<String, Vec<TodoItem>>,
    pub todo_scroll_position: usize,

    // Account usage
    pub account_usage: AccountUsage,

    // UI state
    pub spinner_frame: u8,
    pub current_run_id: u64,

    // Plan modal state (content is re-read from disk on restore, not persisted)
    #[serde(default)]
    pub plan_modal_open: bool,
    #[serde(default)]
    pub plan_modal_scroll: usize,

    // Review history (uses serde(default) for backward compatibility with v4 snapshots)
    #[serde(default)]
    pub review_history: Vec<ReviewRound>,
    #[serde(default)]
    pub review_history_spinner_frame: u8,
    #[serde(default)]
    pub review_history_scroll: usize,
}

/// Information about a session snapshot for listing purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotInfo {
    pub workflow_session_id: String,
    pub feature_name: String,
    pub phase: String,
    pub iteration: u32,
    pub saved_at: String,
    pub working_dir: PathBuf,
}

impl SessionSnapshot {
    /// Creates a snapshot with a specific timestamp (for unified stop operations).
    pub fn new_with_timestamp(
        working_dir: PathBuf,
        workflow_session_id: String,
        state_path: PathBuf,
        workflow_state: State,
        ui_state: SessionUiState,
        total_elapsed_before_resume_ms: u64,
        saved_at: String,
    ) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            saved_at,
            working_dir,
            workflow_session_id,
            state_path,
            workflow_state,
            ui_state,
            total_elapsed_before_resume_ms,
        }
    }

    /// Extracts summary info for listing.
    pub fn info(&self) -> SessionSnapshotInfo {
        SessionSnapshotInfo {
            workflow_session_id: self.workflow_session_id.clone(),
            feature_name: self.workflow_state.feature_name.clone(),
            phase: format!("{:?}", self.workflow_state.phase),
            iteration: self.workflow_state.iteration,
            saved_at: self.saved_at.clone(),
            working_dir: self.working_dir.clone(),
        }
    }
}

/// Returns the home-based sessions directory: `~/.planning-agent/sessions/`
fn get_sessions_dir() -> Result<PathBuf> {
    planning_paths::sessions_dir()
}

/// Returns the snapshot file path for a given session ID.
///
/// Uses the new session-centric path: `~/.planning-agent/sessions/<session-id>/session.json`
fn get_snapshot_path(session_id: &str) -> Result<PathBuf> {
    planning_paths::session_snapshot_path(session_id)
}

/// Returns the legacy snapshot file path: `~/.planning-agent/sessions/<session-id>.json`
fn get_legacy_snapshot_path(session_id: &str) -> Result<PathBuf> {
    planning_paths::snapshot_path(session_id)
}

/// Saves a session snapshot atomically to home storage (`~/.planning-agent/sessions/`).
///
/// Uses the new session-centric path: `~/.planning-agent/sessions/<session-id>/session.json`
///
/// Also creates/updates `session_info.json` for fast session listing.
///
/// The `working_dir` parameter is no longer used for storage location but is kept
/// for API compatibility.
///
/// The snapshot is first written to a temporary file, then renamed to the final path.
#[allow(unused_variables)]
pub fn save_snapshot(working_dir: &Path, snapshot: &SessionSnapshot) -> Result<PathBuf> {
    let snapshot_path = get_snapshot_path(&snapshot.workflow_session_id)?;
    let temp_path = snapshot_path.with_extension("json.tmp");

    let content = serde_json::to_string_pretty(snapshot)
        .context("Failed to serialize session snapshot")?;

    fs::write(&temp_path, &content)
        .with_context(|| format!("Failed to write temp snapshot file: {}", temp_path.display()))?;

    fs::rename(&temp_path, &snapshot_path)
        .with_context(|| format!("Failed to rename temp file to: {}", snapshot_path.display()))?;

    // Also update session_info.json for fast listing
    let _ = update_session_info(snapshot);

    Ok(snapshot_path)
}

/// Updates the session_info.json file for fast session listing.
fn update_session_info(snapshot: &SessionSnapshot) -> Result<()> {
    let info = planning_paths::SessionInfo {
        session_id: snapshot.workflow_session_id.clone(),
        feature_name: snapshot.workflow_state.feature_name.clone(),
        objective: snapshot.workflow_state.objective.clone(),
        working_dir: snapshot.working_dir.clone(),
        created_at: snapshot.saved_at.clone(), // Use saved_at as approximation
        updated_at: snapshot.saved_at.clone(),
        phase: format!("{:?}", snapshot.workflow_state.phase),
        iteration: snapshot.workflow_state.iteration,
    };
    info.save(&snapshot.workflow_session_id)
}

/// Loads a session snapshot by session ID from `~/.planning-agent/sessions/`.
///
/// Checks both new session-centric path and legacy path:
/// 1. New: `~/.planning-agent/sessions/<session-id>/session.json`
/// 2. Legacy: `~/.planning-agent/sessions/<session-id>.json`
///
/// If no snapshot file exists, attempts fallback recovery from the daemon registry
/// and state file. This enables crash recovery when periodic auto-save didn't
/// complete before the crash.
///
/// Note: The `working_dir` parameter is kept for API compatibility but is no longer used.
/// Recovered snapshots are saved using their own stored working_dir to ensure consistency.
#[allow(unused_variables)]
pub fn load_snapshot(working_dir: &Path, session_id: &str) -> Result<SessionSnapshot> {
    // 1. Try new session-centric path first
    let new_path = get_snapshot_path(session_id)?;
    if new_path.exists() {
        return load_snapshot_from_path(&new_path);
    }

    // 2. Try legacy path
    let legacy_path = get_legacy_snapshot_path(session_id)?;
    if legacy_path.exists() {
        return load_snapshot_from_path(&legacy_path);
    }

    // 3. Try fallback recovery from state file + daemon registry
    match recover_from_state_file(session_id) {
        Ok(snapshot) => {
            // Save recovered snapshot for future use (in new format)
            // Use snapshot's own working_dir, not the caller's, to ensure consistency
            if let Err(e) = save_snapshot(&snapshot.working_dir, &snapshot) {
                eprintln!("[recovery] Warning: Failed to save recovered snapshot: {}", e);
            }
            Ok(snapshot)
        }
        Err(e) => {
            anyhow::bail!(
                "Session snapshot not found: {}. Recovery also failed: {}. Use --list-sessions to see available sessions.",
                session_id, e
            );
        }
    }
}

/// Loads a snapshot from a specific path.
fn load_snapshot_from_path(snapshot_path: &Path) -> Result<SessionSnapshot> {
    let content = fs::read_to_string(snapshot_path)
        .with_context(|| format!("Failed to read snapshot file: {}", snapshot_path.display()))?;

    // First, try to parse just the version to determine format
    let version_check: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| "Failed to parse snapshot file as JSON")?;

    let version = version_check
        .get("version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1);

    // Version check
    if version != SNAPSHOT_VERSION {
        anyhow::bail!(
            "Snapshot version {} is not supported (expected {}). Please delete old sessions or upgrade planning-agent.",
            version,
            SNAPSHOT_VERSION
        );
    }

    // Parse as current version
    let snapshot: SessionSnapshot = serde_json::from_str(&content)
        .with_context(|| "Failed to parse snapshot file as JSON")?;

    Ok(snapshot)
}

/// Lists all available session snapshots from `~/.planning-agent/sessions/`.
///
/// Scans both new session-centric structure and legacy structure:
/// 1. New: `~/.planning-agent/sessions/<session-id>/session.json`
/// 2. Legacy: `~/.planning-agent/sessions/<session-id>.json`
#[allow(unused_variables)]
pub fn list_snapshots(working_dir: &Path) -> Result<Vec<SessionSnapshotInfo>> {
    let sessions_dir = get_sessions_dir()?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in fs::read_dir(&sessions_dir)
        .with_context(|| format!("Failed to read sessions directory: {}", sessions_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // New session-centric structure: directory with session.json inside
            let session_json_path = path.join("session.json");
            if session_json_path.exists() {
                if let Ok(content) = fs::read_to_string(&session_json_path) {
                    if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                        if !seen_ids.contains(&snapshot.workflow_session_id) {
                            seen_ids.insert(snapshot.workflow_session_id.clone());
                            snapshots.push(snapshot.info());
                        }
                    }
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "json") {
            // Legacy structure: <session-id>.json file
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                    if !seen_ids.contains(&snapshot.workflow_session_id) {
                        seen_ids.insert(snapshot.workflow_session_id.clone());
                        snapshots.push(snapshot.info());
                    }
                }
            }
        }
    }

    // Sort by saved_at timestamp (newest first)
    snapshots.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));

    Ok(snapshots)
}

/// Deletes a session snapshot from `~/.planning-agent/sessions/`.
///
/// Handles both new and legacy paths. For new session directories,
/// only the session.json file is deleted, not the entire directory
/// (which may contain other files like logs and state).
#[allow(unused_variables)]
pub fn delete_snapshot(working_dir: &Path, session_id: &str) -> Result<()> {
    // Try new path first
    let new_path = get_snapshot_path(session_id)?;
    if new_path.exists() {
        fs::remove_file(&new_path)
            .with_context(|| format!("Failed to delete snapshot: {}", new_path.display()))?;
    }

    // Also try legacy path
    let legacy_path = get_legacy_snapshot_path(session_id)?;
    if legacy_path.exists() {
        fs::remove_file(&legacy_path)
            .with_context(|| format!("Failed to delete snapshot: {}", legacy_path.display()))?;
    }

    Ok(())
}

/// Cleans up session snapshots older than the specified number of days.
///
/// For new session directories, the entire session directory is deleted.
/// For legacy snapshot files, only the .json file is deleted.
/// If the session has a git worktree, it is properly removed first.
pub fn cleanup_old_snapshots(working_dir: &Path, older_than_days: u32) -> Result<Vec<String>> {
    let snapshots = list_snapshots(working_dir)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(older_than_days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let mut deleted = Vec::new();

    for snapshot in snapshots {
        if snapshot.saved_at < cutoff_str {
            let session_id = &snapshot.workflow_session_id;

            // Load the full snapshot to get worktree info
            if let Ok(full_snapshot) = load_snapshot(working_dir, session_id) {
                // Clean up git worktree if present
                if let Some(ref wt_state) = full_snapshot.workflow_state.worktree_info {
                    if wt_state.worktree_path.exists() {
                        if let Err(e) = crate::git_worktree::remove_worktree(
                            &wt_state.original_dir,
                            &wt_state.worktree_path,
                            Some(&wt_state.branch_name),
                        ) {
                            eprintln!("[cleanup] Warning: Failed to remove worktree: {}", e);
                            // Continue anyway - we'll still try to delete the directory
                        }
                    }
                }
            }

            // Check if this is a new-style session directory
            if let Ok(session_dir) = planning_paths::session_dir(session_id) {
                if session_dir.exists() && session_dir.is_dir() {
                    // Check if session.json exists inside (confirms new structure)
                    if session_dir.join("session.json").exists() {
                        // Delete the entire session directory
                        if let Err(e) = fs::remove_dir_all(&session_dir) {
                            eprintln!("[cleanup] Failed to delete session directory {}: {}", session_dir.display(), e);
                        } else {
                            deleted.push(session_id.clone());
                            continue;
                        }
                    }
                }
            }

            // Fall back to deleting just the snapshot file (legacy)
            delete_snapshot(working_dir, session_id)?;
            deleted.push(session_id.clone());
        }
    }

    Ok(deleted)
}

/// Checks for potential conflict between snapshot and state file.
///
/// Returns `Some(state_updated_at)` if there's a conflict (state file is newer than snapshot).
/// Returns `None` if there's no conflict or conflict detection should be skipped.
pub fn check_conflict(snapshot: &SessionSnapshot, current_state: &State) -> Option<String> {
    // Skip conflict detection for legacy state files without updated_at
    if !current_state.has_updated_at() {
        return None;
    }

    // Compare snapshot.saved_at with state.updated_at
    if current_state.updated_at > snapshot.saved_at {
        Some(current_state.updated_at.clone())
    } else {
        None
    }
}

/// Attempts to recover a session from state file when no snapshot exists.
/// Returns a minimal snapshot with just workflow state (no UI state).
///
/// This is a fallback mechanism for crash recovery when the periodic auto-save
/// didn't save a snapshot before the crash.
///
/// Recovery order:
/// 1. Try new session-centric state path: `~/.planning-agent/sessions/<session-id>/state.json`
/// 2. Fall back to daemon registry to find legacy state path
pub fn recover_from_state_file(session_id: &str) -> Result<SessionSnapshot> {
    // 1. Try new session-centric state path first
    if let Ok(session_state_path) = planning_paths::session_state_path(session_id) {
        if session_state_path.exists() {
            let state = State::load(&session_state_path)
                .with_context(|| format!(
                    "Failed to load state file at {}. The file may be corrupted.",
                    session_state_path.display()
                ))?;

            let ui_state = SessionUiState::minimal_from_state(&state);

            // Get working_dir from state's plan_file parent or use current dir
            let working_dir = state.plan_file.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

            return Ok(SessionSnapshot::new_with_timestamp(
                working_dir,
                session_id.to_string(),
                session_state_path,
                state,
                ui_state,
                0,
                chrono::Utc::now().to_rfc3339(),
            ));
        }
    }

    // 2. Fall back to daemon registry for legacy state paths
    let registry_path = planning_paths::sessiond_registry_path()?;
    if !registry_path.exists() {
        anyhow::bail!("No daemon registry found for recovery");
    }

    let content = fs::read_to_string(&registry_path)
        .context("Failed to read daemon registry")?;

    // NOTE: Registry format is Vec<SessionRecord> (JSON array), verified in server.rs
    let records: Vec<crate::session_daemon::protocol::SessionRecord> =
        serde_json::from_str(&content)
            .context("Failed to parse daemon registry")?;

    let record = records.iter()
        .find(|r| r.workflow_session_id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session {} not found in daemon registry", session_id))?;

    // 3. Check if state file exists (may have been deleted)
    if !record.state_path.exists() {
        anyhow::bail!(
            "State file not found at {}. The session data may have been deleted.",
            record.state_path.display()
        );
    }

    // 4. Load state from state_path
    let state = State::load(&record.state_path)
        .with_context(|| format!(
            "Failed to load state file at {}. The file may be corrupted.",
            record.state_path.display()
        ))?;

    // 4. Create minimal UI state with defaults
    let ui_state = SessionUiState::minimal_from_state(&state);

    // 5. Build snapshot
    Ok(SessionSnapshot::new_with_timestamp(
        record.working_dir.clone(),
        session_id.to_string(),
        record.state_path.clone(),
        state,
        ui_state,
        0,
        chrono::Utc::now().to_rfc3339(),
    ))
}

impl SessionUiState {
    /// Create minimal UI state from workflow state (for recovery from crashes).
    /// All UI state is reset to defaults since we don't have the original.
    pub fn minimal_from_state(state: &State) -> Self {
        use crate::state::Phase;
        Self {
            id: 0,
            name: state.feature_name.clone(),
            status: match state.phase {
                Phase::Planning => SessionStatus::Planning,
                Phase::Reviewing => SessionStatus::AwaitingApproval,
                Phase::Revising => SessionStatus::Planning,
                Phase::Complete => SessionStatus::Complete,
            },
            output_lines: vec![
                "[recovery] Session recovered from state file".to_string(),
                format!("[recovery] Phase: {:?}, Iteration: {}", state.phase, state.iteration),
                "[recovery] UI state reset to defaults. Workflow state preserved.".to_string(),
            ],
            scroll_position: 0,
            output_follow_mode: true,
            streaming_lines: Vec::new(),
            streaming_scroll_position: 0,
            streaming_follow_mode: true,
            focused_panel: FocusedPanel::Output,
            total_cost: 0.0,
            bytes_received: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            tool_call_count: 0,
            bytes_per_second: 0.0,
            turn_count: 0,
            model_name: None,
            last_stop_reason: None,
            tool_error_count: 0,
            total_tool_duration_ms: 0,
            completed_tool_count: 0,
            approval_mode: ApprovalMode::None,
            approval_context: ApprovalContext::PlanApproval,
            plan_summary: String::new(),
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,
            feedback_scroll: 0,
            feedback_target: FeedbackTarget::ApprovalDecline,
            input_mode: InputMode::Normal,
            tab_input: String::new(),
            tab_input_cursor: 0,
            tab_input_scroll: 0,
            last_key_was_backslash: false,
            tab_input_pastes: Vec::new(),
            feedback_pastes: Vec::new(),
            error_state: None,
            error_scroll: 0,
            run_tabs: Vec::new(),
            active_run_tab: 0,
            chat_follow_mode: true,
            todos: HashMap::new(),
            todo_scroll_position: 0,
            account_usage: AccountUsage::default(),
            spinner_frame: 0,
            current_run_id: 0,
            plan_modal_open: false,
            plan_modal_scroll: 0,
            review_history: Vec::new(),
            review_history_spinner_frame: 0,
            review_history_scroll: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> State {
        State::new("test-feature", "Test objective", 3).unwrap()
    }

    fn create_test_ui_state() -> SessionUiState {
        SessionUiState {
            id: 1,
            name: "Test Session".to_string(),
            status: SessionStatus::Planning,
            output_lines: vec!["line1".to_string(), "line2".to_string()],
            scroll_position: 0,
            output_follow_mode: true,
            streaming_lines: Vec::new(),
            streaming_scroll_position: 0,
            streaming_follow_mode: true,
            focused_panel: FocusedPanel::Output,
            total_cost: 0.0,
            bytes_received: 100,
            total_input_tokens: 50,
            total_output_tokens: 30,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            tool_call_count: 5,
            bytes_per_second: 100.0,
            turn_count: 2,
            model_name: Some("claude".to_string()),
            last_stop_reason: None,
            tool_error_count: 0,
            total_tool_duration_ms: 1000,
            completed_tool_count: 5,
            approval_mode: ApprovalMode::None,
            approval_context: ApprovalContext::PlanApproval,
            plan_summary: "Test plan".to_string(),
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,
            feedback_scroll: 0,
            feedback_target: FeedbackTarget::ApprovalDecline,
            input_mode: InputMode::Normal,
            tab_input: String::new(),
            tab_input_cursor: 0,
            tab_input_scroll: 0,
            last_key_was_backslash: false,
            tab_input_pastes: Vec::new(),
            feedback_pastes: Vec::new(),
            error_state: None,
            error_scroll: 0,
            run_tabs: Vec::new(),
            active_run_tab: 0,
            chat_follow_mode: true,
            todos: HashMap::new(),
            todo_scroll_position: 0,
            account_usage: AccountUsage::default(),
            spinner_frame: 0,
            current_run_id: 1,
            plan_modal_open: false,
            plan_modal_scroll: 0,
            review_history: Vec::new(),
            review_history_spinner_frame: 0,
            review_history_scroll: 0,
        }
    }

    #[test]
    fn test_snapshot_creation() {
        let state = create_test_state();
        let ui_state = create_test_ui_state();
        let saved_at = chrono::Utc::now().to_rfc3339();

        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state.clone(),
            ui_state,
            0,
            saved_at,
        );

        assert_eq!(snapshot.version, SNAPSHOT_VERSION);
        assert!(!snapshot.saved_at.is_empty());
        assert_eq!(snapshot.workflow_session_id, "test-session-id");
        assert_eq!(snapshot.workflow_state.feature_name, "test-feature");
    }

    #[test]
    fn test_snapshot_serialization_roundtrip() {
        let state = create_test_state();
        let ui_state = create_test_ui_state();
        let saved_at = chrono::Utc::now().to_rfc3339();

        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state,
            ui_state,
            5000,
            saved_at,
        );

        let json = serde_json::to_string(&snapshot).unwrap();
        let loaded: SessionSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.version, snapshot.version);
        assert_eq!(loaded.saved_at, snapshot.saved_at);
        assert_eq!(loaded.workflow_session_id, snapshot.workflow_session_id);
        assert_eq!(loaded.workflow_state.feature_name, snapshot.workflow_state.feature_name);
        assert_eq!(loaded.ui_state.name, snapshot.ui_state.name);
        assert_eq!(loaded.total_elapsed_before_resume_ms, 5000);
    }

    #[test]
    fn test_snapshot_info() {
        let state = create_test_state();
        let ui_state = create_test_ui_state();
        let saved_at = chrono::Utc::now().to_rfc3339();

        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state,
            ui_state,
            0,
            saved_at,
        );

        let info = snapshot.info();
        assert_eq!(info.workflow_session_id, "test-session-id");
        assert_eq!(info.feature_name, "test-feature");
        assert_eq!(info.phase, "Planning");
        assert_eq!(info.iteration, 1);
    }

    #[test]
    fn test_conflict_detection_no_conflict() {
        let mut state = create_test_state();
        state.set_updated_at_with("2025-12-29T14:00:00Z");

        let ui_state = create_test_ui_state();
        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state.clone(),
            ui_state,
            0,
            "2025-12-29T15:00:00Z".to_string(), // Snapshot saved after state update
        );

        // No conflict - snapshot is newer than state
        let conflict = check_conflict(&snapshot, &state);
        assert!(conflict.is_none());
    }

    #[test]
    fn test_conflict_detection_with_conflict() {
        let mut state = create_test_state();
        state.set_updated_at_with("2025-12-29T16:00:00Z"); // State updated after snapshot

        let ui_state = create_test_ui_state();
        let original_state = create_test_state();
        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            original_state,
            ui_state,
            0,
            "2025-12-29T15:00:00Z".to_string(), // Snapshot saved before state update
        );

        // Conflict - state is newer than snapshot
        let conflict = check_conflict(&snapshot, &state);
        assert!(conflict.is_some());
        assert_eq!(conflict.unwrap(), "2025-12-29T16:00:00Z");
    }

    #[test]
    fn test_conflict_detection_skipped_for_legacy_state() {
        let mut state = create_test_state();
        state.updated_at = String::new(); // Simulate legacy state without updated_at

        let ui_state = create_test_ui_state();
        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state.clone(),
            ui_state,
            0,
            chrono::Utc::now().to_rfc3339(),
        );

        // No conflict detection for legacy state files
        let conflict = check_conflict(&snapshot, &state);
        assert!(conflict.is_none());
    }

    #[test]
    fn test_session_centric_snapshot_path() {
        // Test that get_snapshot_path returns session-centric path
        if std::env::var("HOME").is_err() {
            return;
        }

        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        let path = get_snapshot_path(&session_id).unwrap();

        // Should be ~/.planning-agent/sessions/<session-id>/session.json
        assert!(path.to_string_lossy().contains(".planning-agent/sessions/"));
        assert!(path.to_string_lossy().contains(&session_id));
        assert!(path.to_string_lossy().ends_with("/session.json"));
    }

    #[test]
    fn test_legacy_snapshot_path() {
        // Test that get_legacy_snapshot_path returns legacy path
        if std::env::var("HOME").is_err() {
            return;
        }

        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        let path = get_legacy_snapshot_path(&session_id).unwrap();

        // Should be ~/.planning-agent/sessions/<session-id>.json
        assert!(path.to_string_lossy().contains(".planning-agent/sessions/"));
        assert!(path.to_string_lossy().ends_with(".json"));
        assert!(!path.to_string_lossy().contains("/session.json"));
    }

    #[test]
    fn test_save_and_load_session_centric_snapshot() {
        if std::env::var("HOME").is_err() {
            return;
        }

        let state = create_test_state();
        let ui_state = create_test_ui_state();
        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        let saved_at = chrono::Utc::now().to_rfc3339();

        let snapshot = SessionSnapshot::new_with_timestamp(
            PathBuf::from("/tmp/test"),
            session_id.clone(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state,
            ui_state,
            0,
            saved_at,
        );

        // Save
        let save_result = save_snapshot(Path::new("/tmp"), &snapshot);
        assert!(save_result.is_ok());
        let saved_path = save_result.unwrap();

        // Verify session-centric path
        assert!(saved_path.to_string_lossy().contains(&session_id));
        assert!(saved_path.to_string_lossy().ends_with("/session.json"));

        // Load
        let load_result = load_snapshot(Path::new("/tmp"), &session_id);
        assert!(load_result.is_ok());
        let loaded = load_result.unwrap();

        assert_eq!(loaded.workflow_session_id, session_id);
        assert_eq!(loaded.workflow_state.feature_name, "test-feature");

        // Cleanup
        let _ = delete_snapshot(Path::new("/tmp"), &session_id);
    }
}
