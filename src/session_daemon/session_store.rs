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

use crate::app::AccountUsage;
use crate::domain::types::Phase;
use crate::domain::view::WorkflowView;
use crate::planning_paths;
use crate::tui::scroll::ScrollState;
use crate::tui::session::model::{
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, PasteBlock,
    ReviewRound, RunTab, SessionStatus, TodoItem,
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
/// - v5: Review rounds include a kind discriminator (plan vs implementation)
/// - v6: Added workflow_name to preserve workflow config across resume
/// - v7: Added workflow_view and last_event_sequence for CQRS event sourcing
pub const SNAPSHOT_VERSION: u32 = 7;

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
    /// The workflow session ID (from WorkflowView.workflow_id)
    pub workflow_session_id: String,
    /// Path to the state file for this workflow
    pub state_path: PathBuf,
    /// UI state that can be restored
    pub ui_state: SessionUiState,
    /// Total elapsed time before this resume (milliseconds).
    /// Accumulated across multiple stop/resume cycles.
    pub total_elapsed_before_resume_ms: u64,
    /// Name of the workflow used for this session (e.g., "claude-only", "default").
    /// Used to restore the correct workflow config on resume.
    pub workflow_name: String,
    /// Event-sourced view of the workflow (CQRS projection).
    /// This is the authoritative source for workflow state.
    pub workflow_view: WorkflowView,
    /// Last applied event sequence number for resumption.
    /// Events after this sequence need to be replayed on resume.
    #[serde(default)]
    pub last_event_sequence: u64,
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
    #[serde(default)]
    pub output_scroll: ScrollState,
    pub streaming_lines: Vec<String>,
    #[serde(default)]
    pub streaming_scroll: ScrollState,
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
    pub todos: HashMap<String, Vec<TodoItem>>,
    #[serde(default)]
    pub todo_scroll: ScrollState,

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

    // Review modal state (content is re-read from disk on restore, not persisted)
    #[serde(default)]
    pub review_modal_open: bool,
    #[serde(default)]
    pub review_modal_scroll: usize,
    #[serde(default)]
    pub review_modal_tab: usize,

    // Review history (uses serde(default) for backward compatibility with v4 snapshots)
    #[serde(default)]
    pub review_history: Vec<ReviewRound>,
    #[serde(default)]
    pub review_history_spinner_frame: u8,
    #[serde(default)]
    pub review_history_scroll: ScrollState,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_timestamp(
        working_dir: PathBuf,
        workflow_session_id: String,
        state_path: PathBuf,
        ui_state: SessionUiState,
        total_elapsed_before_resume_ms: u64,
        saved_at: String,
        workflow_name: String,
        workflow_view: WorkflowView,
        last_event_sequence: u64,
    ) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            saved_at,
            working_dir,
            workflow_session_id,
            state_path,
            ui_state,
            total_elapsed_before_resume_ms,
            workflow_name,
            workflow_view,
            last_event_sequence,
        }
    }

    /// Extracts summary info for listing.
    pub fn info(&self) -> SessionSnapshotInfo {
        SessionSnapshotInfo {
            workflow_session_id: self.workflow_session_id.clone(),
            feature_name: self
                .workflow_view
                .feature_name()
                .map(|f| f.0.clone())
                .unwrap_or_default(),
            phase: format!(
                "{:?}",
                self.workflow_view
                    .planning_phase()
                    .unwrap_or(Phase::Planning)
            ),
            iteration: self.workflow_view.iteration().map(|i| i.0).unwrap_or(1),
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
pub fn get_snapshot_path(session_id: &str) -> Result<PathBuf> {
    planning_paths::session_snapshot_path(session_id)
}

/// Saves a session snapshot atomically to home storage (`~/.planning-agent/sessions/`).
///
/// Uses the session-centric path: `~/.planning-agent/sessions/<session-id>/session.json`
///
/// Also creates/updates `session_info.json` for fast session listing.
///
/// The snapshot is first written to a temporary file, then renamed to the final path.
pub fn save_snapshot(snapshot: &SessionSnapshot) -> Result<PathBuf> {
    let snapshot_path = get_snapshot_path(&snapshot.workflow_session_id)?;
    let temp_path = snapshot_path.with_extension("json.tmp");

    let content =
        serde_json::to_string_pretty(snapshot).context("Failed to serialize session snapshot")?;

    fs::write(&temp_path, &content).with_context(|| {
        format!(
            "Failed to write temp snapshot file: {}",
            temp_path.display()
        )
    })?;

    fs::rename(&temp_path, &snapshot_path)
        .with_context(|| format!("Failed to rename temp file to: {}", snapshot_path.display()))?;

    // Also update session_info.json for fast listing.
    // Best-effort: the main snapshot was saved successfully, so failure here is non-critical.
    // The info file is an optimization for listing, not essential for resume.
    let _ = update_session_info(snapshot);

    Ok(snapshot_path)
}

/// Updates the session_info.json file for fast session listing.
fn update_session_info(snapshot: &SessionSnapshot) -> Result<()> {
    let info = planning_paths::SessionInfo {
        session_id: snapshot.workflow_session_id.clone(),
        feature_name: snapshot
            .workflow_view
            .feature_name()
            .map(|f| f.0.clone())
            .unwrap_or_default(),
        objective: snapshot
            .workflow_view
            .objective()
            .map(|o| o.0.clone())
            .unwrap_or_default(),
        working_dir: snapshot.working_dir.clone(),
        created_at: snapshot.saved_at.clone(), // Use saved_at as approximation
        updated_at: snapshot.saved_at.clone(),
        phase: format!(
            "{:?}",
            snapshot
                .workflow_view
                .planning_phase()
                .unwrap_or(Phase::Planning)
        ),
        iteration: snapshot.workflow_view.iteration().map(|i| i.0).unwrap_or(1),
    };
    info.save(&snapshot.workflow_session_id)
}

/// Loads a session snapshot by session ID from `~/.planning-agent/sessions/`.
///
/// Loads from: `~/.planning-agent/sessions/<session-id>/session.json`
///
/// If no snapshot file exists, attempts fallback recovery from the daemon registry
/// and state file. This enables crash recovery when periodic auto-save didn't
/// complete before the crash.
pub fn load_snapshot(session_id: &str) -> Result<SessionSnapshot> {
    let snapshot_path = get_snapshot_path(session_id)?;

    if snapshot_path.exists() {
        return load_snapshot_from_path(&snapshot_path);
    }

    // Try fallback recovery from state file + daemon registry
    match recover_from_state_file(session_id) {
        Ok(snapshot) => {
            if let Err(e) = save_snapshot(&snapshot) {
                eprintln!(
                    "[recovery] Warning: Failed to save recovered snapshot: {}",
                    e
                );
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
    let version_check: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Failed to parse snapshot file as JSON")?;

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
    let snapshot: SessionSnapshot =
        serde_json::from_str(&content).with_context(|| "Failed to parse snapshot file as JSON")?;

    Ok(snapshot)
}

/// Lists all available session snapshots from `~/.planning-agent/sessions/`.
///
/// Scans session directories: `~/.planning-agent/sessions/<session-id>/session.json`
pub fn list_snapshots() -> Result<Vec<SessionSnapshotInfo>> {
    let sessions_dir = get_sessions_dir()?;

    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();

    for entry in fs::read_dir(&sessions_dir).with_context(|| {
        format!(
            "Failed to read sessions directory: {}",
            sessions_dir.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let session_json_path = path.join("session.json");
            if session_json_path.exists() {
                if let Ok(content) = fs::read_to_string(&session_json_path) {
                    if let Ok(snapshot) = serde_json::from_str::<SessionSnapshot>(&content) {
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
/// Only the session.json file is deleted, not the entire session directory
/// (which may contain other files like logs and state).
#[cfg(test)]
pub fn delete_snapshot(session_id: &str) -> Result<()> {
    let snapshot_path = get_snapshot_path(session_id)?;
    if snapshot_path.exists() {
        fs::remove_file(&snapshot_path)
            .with_context(|| format!("Failed to delete snapshot: {}", snapshot_path.display()))?;
    }

    Ok(())
}

/// Cleans up session snapshots older than the specified number of days.
///
/// The entire session directory is deleted.
/// If the session has a git worktree, it is properly removed first.
pub fn cleanup_old_snapshots(older_than_days: u32) -> Result<Vec<String>> {
    let snapshots = list_snapshots()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(older_than_days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    let mut deleted = Vec::new();

    for snapshot in snapshots {
        if snapshot.saved_at < cutoff_str {
            let session_id = &snapshot.workflow_session_id;

            // Load the full snapshot to get worktree info
            if let Ok(full_snapshot) = load_snapshot(session_id) {
                // Clean up git worktree if present
                if let Some(wt_state) = full_snapshot.workflow_view.worktree_info() {
                    if wt_state.worktree_path().exists() {
                        if let Err(e) = crate::git_worktree::remove_worktree(
                            wt_state.original_dir(),
                            wt_state.worktree_path(),
                            Some(wt_state.branch_name()),
                        ) {
                            eprintln!("[cleanup] Warning: Failed to remove worktree: {}", e);
                            // Continue anyway - we'll still try to delete the directory
                        }
                    }
                }
            }

            // Delete the entire session directory
            if let Ok(session_dir) = planning_paths::session_dir(session_id) {
                if session_dir.exists() && session_dir.is_dir() {
                    if let Err(e) = fs::remove_dir_all(&session_dir) {
                        eprintln!(
                            "[cleanup] Failed to delete session directory {}: {}",
                            session_dir.display(),
                            e
                        );
                    } else {
                        deleted.push(session_id.clone());
                    }
                }
            }
        }
    }

    Ok(deleted)
}

/// Attempts to recover a session from event log when no snapshot exists.
/// Returns a minimal snapshot with just workflow state (no UI state).
///
/// This is a fallback mechanism for crash recovery when the periodic auto-save
/// didn't save a snapshot before the crash.
///
/// Recovery order:
/// 1. Try to bootstrap WorkflowView from event log
/// 2. Fall back to daemon registry to find working directory
pub fn recover_from_state_file(session_id: &str) -> Result<SessionSnapshot> {
    // 1. Try to bootstrap WorkflowView from event log
    let log_path = planning_paths::session_event_log_path(session_id)?;
    if !log_path.exists() {
        anyhow::bail!(
            "Event log not found at {}. The session data may have been deleted.",
            log_path.display()
        );
    }

    let workflow_view = crate::domain::actor::bootstrap_view_from_events(&log_path, session_id);
    let last_event_sequence = workflow_view.last_event_sequence();

    if last_event_sequence == 0 {
        anyhow::bail!(
            "Event log at {} is empty or could not be parsed.",
            log_path.display()
        );
    }

    // Get working_dir from workflow_view or fall back to daemon registry
    let working_dir = workflow_view
        .working_dir()
        .map(|w| w.0.clone())
        .or_else(|| {
            // Fall back to daemon registry
            let registry_path = planning_paths::sessiond_registry_path().ok()?;
            let content = fs::read_to_string(&registry_path).ok()?;
            let records: Vec<crate::session_daemon::protocol::SessionRecord> =
                serde_json::from_str(&content).ok()?;
            records
                .iter()
                .find(|r| r.workflow_session_id == session_id)
                .map(|r| r.working_dir.clone())
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Get state_path from session directory
    let state_path = planning_paths::session_state_path(session_id)?;

    // Create minimal UI state from WorkflowView
    let ui_state = SessionUiState::minimal_from_view(&workflow_view);

    // For recovery, we don't know the original workflow name, so use the
    // current selection for this working directory (best effort)
    let workflow_name = crate::app::WorkflowSelection::load(&working_dir)
        .map(|s| s.workflow)
        .unwrap_or_else(|_| "claude-only".to_string());

    // Build snapshot
    Ok(SessionSnapshot::new_with_timestamp(
        working_dir,
        session_id.to_string(),
        state_path,
        ui_state,
        0,
        chrono::Utc::now().to_rfc3339(),
        workflow_name,
        workflow_view,
        last_event_sequence,
    ))
}

impl SessionUiState {
    /// Create minimal UI state from WorkflowView (for recovery from crashes).
    /// All UI state is reset to defaults since we don't have the original.
    pub fn minimal_from_view(view: &WorkflowView) -> Self {
        let phase = view.planning_phase().unwrap_or(Phase::Planning);
        let iteration = view.iteration().map(|i| i.0).unwrap_or(1);
        let feature_name = view.feature_name().map(|f| f.0.clone()).unwrap_or_default();

        Self {
            id: 0,
            name: feature_name,
            status: match phase {
                Phase::Planning => SessionStatus::Planning,
                Phase::Reviewing => SessionStatus::AwaitingApproval,
                Phase::Revising => SessionStatus::Planning,
                Phase::AwaitingPlanningDecision => SessionStatus::AwaitingApproval,
                Phase::Complete => SessionStatus::Complete,
            },
            output_lines: vec![
                "[recovery] Session recovered from event log".to_string(),
                format!("[recovery] Phase: {:?}, Iteration: {}", phase, iteration),
                "[recovery] UI state reset to defaults. Workflow state preserved.".to_string(),
            ],
            output_scroll: ScrollState::new(),
            streaming_lines: Vec::new(),
            streaming_scroll: ScrollState::new(),
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
            todos: HashMap::new(),
            todo_scroll: ScrollState::new(),
            account_usage: AccountUsage::default(),
            spinner_frame: 0,
            current_run_id: 0,
            plan_modal_open: false,
            plan_modal_scroll: 0,
            review_modal_open: false,
            review_modal_scroll: 0,
            review_modal_tab: 0,
            review_history: Vec::new(),
            review_history_spinner_frame: 0,
            review_history_scroll: ScrollState::new(),
        }
    }
}

#[cfg(test)]
#[path = "tests/session_store_tests.rs"]
mod tests;
