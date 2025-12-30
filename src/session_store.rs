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
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, PasteBlock, RunTab,
    SessionStatus, TodoItem,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Current snapshot format version.
/// Increment this when making breaking changes to the snapshot format.
pub const SNAPSHOT_VERSION: u32 = 1;

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
    /// Creates a new snapshot with the current timestamp.
    pub fn new(
        working_dir: PathBuf,
        workflow_session_id: String,
        state_path: PathBuf,
        workflow_state: State,
        ui_state: SessionUiState,
        total_elapsed_before_resume_ms: u64,
    ) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            saved_at: chrono::Utc::now().to_rfc3339(),
            working_dir,
            workflow_session_id,
            state_path,
            workflow_state,
            ui_state,
            total_elapsed_before_resume_ms,
        }
    }

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

/// Returns the snapshot file path for a given session ID (home-based).
fn get_snapshot_path(session_id: &str) -> Result<PathBuf> {
    planning_paths::snapshot_path(session_id)
}

/// Saves a session snapshot atomically to home storage (`~/.planning-agent/sessions/`).
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

    Ok(snapshot_path)
}

/// Loads a session snapshot by session ID from `~/.planning-agent/sessions/`.
#[allow(unused_variables)]
pub fn load_snapshot(working_dir: &Path, session_id: &str) -> Result<SessionSnapshot> {
    let snapshot_path = get_snapshot_path(session_id)?;

    if !snapshot_path.exists() {
        anyhow::bail!(
            "Session snapshot not found: {}. Use --list-sessions to see available sessions.",
            session_id
        );
    }

    let content = fs::read_to_string(&snapshot_path)
        .with_context(|| format!("Failed to read snapshot file: {}", snapshot_path.display()))?;

    let snapshot: SessionSnapshot = serde_json::from_str(&content)
        .with_context(|| "Failed to parse snapshot file as JSON")?;

    // Version check
    if snapshot.version > SNAPSHOT_VERSION {
        anyhow::bail!(
            "Snapshot version {} is newer than supported version {}. Please upgrade planning-agent.",
            snapshot.version,
            SNAPSHOT_VERSION
        );
    }

    Ok(snapshot)
}

/// Lists all available session snapshots from `~/.planning-agent/sessions/`.
#[allow(unused_variables)]
pub fn list_snapshots(working_dir: &Path) -> Result<Vec<SessionSnapshotInfo>> {
    let sessions_dir = get_sessions_dir()?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();

    for entry in fs::read_dir(&sessions_dir)
        .with_context(|| format!("Failed to read sessions directory: {}", sessions_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map_or(false, |ext| ext == "json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
                    snapshots.push(snapshot.info());
                }
            }
        }
    }

    // Sort by saved_at timestamp (newest first)
    snapshots.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));

    Ok(snapshots)
}

/// Deletes a session snapshot from `~/.planning-agent/sessions/`.
#[allow(unused_variables)]
pub fn delete_snapshot(working_dir: &Path, session_id: &str) -> Result<()> {
    let snapshot_path = get_snapshot_path(session_id)?;

    if snapshot_path.exists() {
        fs::remove_file(&snapshot_path)
            .with_context(|| format!("Failed to delete snapshot: {}", snapshot_path.display()))?;
    }

    Ok(())
}

/// Cleans up session snapshots older than the specified number of days.
pub fn cleanup_old_snapshots(working_dir: &Path, older_than_days: u32) -> Result<Vec<String>> {
    let snapshots = list_snapshots(working_dir)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(older_than_days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let mut deleted = Vec::new();

    for snapshot in snapshots {
        if snapshot.saved_at < cutoff_str {
            delete_snapshot(working_dir, &snapshot.workflow_session_id)?;
            deleted.push(snapshot.workflow_session_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Phase;

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
        }
    }

    #[test]
    fn test_snapshot_creation() {
        let state = create_test_state();
        let ui_state = create_test_ui_state();

        let snapshot = SessionSnapshot::new(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state.clone(),
            ui_state,
            0,
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

        let snapshot = SessionSnapshot::new(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state,
            ui_state,
            5000,
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

        let snapshot = SessionSnapshot::new(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state,
            ui_state,
            0,
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
        let snapshot = SessionSnapshot::new(
            PathBuf::from("/tmp/test"),
            "test-session-id".to_string(),
            PathBuf::from("/tmp/test/.planning-agent/test-feature.json"),
            state.clone(),
            ui_state,
            0,
        );

        // No conflict detection for legacy state files
        let conflict = check_conflict(&snapshot, &state);
        assert!(conflict.is_none());
    }
}
