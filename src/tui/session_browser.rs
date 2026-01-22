//! Session browser overlay for viewing and resuming workflow sessions.
//!
//! This module provides a modal overlay that displays sessions from both
//! the live daemon registry and disk snapshots, allowing users to:
//! - View running and stopped sessions with live status updates
//! - Resume stopped sessions in new tabs or terminals
//! - Force-stop unresponsive sessions
//! - Filter sessions by working directory

use crate::session_daemon::{self, LivenessState, SessionRecord};
use crate::session_store::{list_snapshots, SessionSnapshotInfo};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

/// Confirmation state for pending user actions.
#[derive(Debug, Clone)]
pub enum ConfirmationState {
    /// Confirm resuming a session in a different working directory
    CrossDirectoryResume {
        session_id: String,
        target_dir: PathBuf,
    },
    /// Confirm force-stopping a running/unresponsive session
    ForceStop { session_id: String },
}

/// A session entry in the browser list, merging live and snapshot data.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionEntry {
    /// The workflow session ID
    pub session_id: String,
    /// Feature name for display
    pub feature_name: String,
    /// Current workflow phase
    pub phase: String,
    /// Current iteration number
    pub iteration: u32,
    /// Workflow status (e.g., "Planning", "Complete", "Error")
    pub workflow_status: String,
    /// Liveness state from daemon (Running, Unresponsive, Stopped)
    pub liveness: LivenessState,
    /// Last update/heartbeat timestamp (RFC3339)
    pub last_seen_at: String,
    /// Relative time string for display (e.g., "2m ago")
    pub last_seen_relative: String,
    /// Working directory
    pub working_dir: PathBuf,
    /// Whether the snapshot is from the current working directory
    pub is_current_dir: bool,
    /// Whether this entry has a snapshot file (required for resume)
    pub has_snapshot: bool,
    /// Whether this session can be resumed (has snapshot AND not Running)
    pub is_resumable: bool,
    /// PID of the owning process (if live)
    pub pid: Option<u32>,
    /// Whether this entry came from live daemon data
    pub is_live: bool,
}

impl SessionEntry {
    /// Creates an entry from a snapshot (stopped session).
    fn from_snapshot(snapshot: &SessionSnapshotInfo, is_current_dir: bool) -> Self {
        let last_seen_relative = format_relative_time(&snapshot.saved_at);
        Self {
            session_id: snapshot.workflow_session_id.clone(),
            feature_name: snapshot.feature_name.clone(),
            phase: snapshot.phase.clone(),
            iteration: snapshot.iteration,
            workflow_status: "Stopped".to_string(),
            liveness: LivenessState::Stopped,
            last_seen_at: snapshot.saved_at.clone(),
            last_seen_relative,
            working_dir: snapshot.working_dir.clone(),
            is_current_dir,
            has_snapshot: true,
            is_resumable: true, // Snapshots are always resumable
            pid: None,
            is_live: false,
        }
    }

    /// Creates an entry from a live daemon session record.
    fn from_live(record: &SessionRecord, is_current_dir: bool, has_snapshot: bool) -> Self {
        let last_seen_relative = format_relative_time(&record.last_heartbeat_at);
        let is_resumable = has_snapshot && record.liveness != LivenessState::Running;
        Self {
            session_id: record.workflow_session_id.clone(),
            feature_name: record.feature_name.clone(),
            phase: record.phase.clone(),
            iteration: record.iteration,
            workflow_status: record.workflow_status.clone(),
            liveness: record.liveness,
            last_seen_at: record.last_heartbeat_at.clone(),
            last_seen_relative,
            working_dir: record.working_dir.clone(),
            is_current_dir,
            has_snapshot,
            is_resumable,
            pid: Some(record.pid),
            is_live: true,
        }
    }
}

/// Formats a timestamp as relative time (e.g., "2m ago", "1h ago").
fn format_relative_time(timestamp: &str) -> String {
    // Try RFC3339 first
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            // Try parsing without timezone
            chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .or_else(|| {
                    timestamp.get(..19).and_then(|s| {
                        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok()
                    })
                })
                .map(|dt| dt.and_utc())
        });

    match parsed {
        Some(dt) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt);

            if duration.num_seconds() < 60 {
                "just now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{}m ago", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{}h ago", duration.num_hours())
            } else {
                format!("{}d ago", duration.num_days())
            }
        }
        None => "unknown".to_string(),
    }
}

/// State for the session browser overlay.
#[derive(Debug, Clone)]
pub struct SessionBrowserState {
    /// Whether the overlay is open
    pub open: bool,
    /// List of session entries (merged live + snapshots)
    pub entries: Vec<SessionEntry>,
    /// Currently selected index
    pub selected_idx: usize,
    /// Scroll offset for the list
    pub scroll_offset: usize,
    /// Filter: show only current directory sessions
    pub filter_current_dir: bool,
    /// Error message if loading failed
    pub error: Option<String>,
    /// Whether we're in the process of resuming
    pub resuming: bool,
    /// Pending confirmation action
    pub confirmation_pending: Option<ConfirmationState>,
    /// Last refresh timestamp
    pub last_refresh_at: Option<Instant>,
    /// Whether a refresh is in progress
    pub loading: bool,
    /// Whether daemon is connected (for graceful degradation notice)
    pub daemon_connected: bool,
    /// Current working directory (cached for filtering)
    pub current_working_dir: PathBuf,
}

impl Default for SessionBrowserState {
    fn default() -> Self {
        Self {
            open: false,
            entries: Vec::new(),
            selected_idx: 0,
            scroll_offset: 0,
            filter_current_dir: false,
            error: None,
            resuming: false,
            confirmation_pending: None,
            last_refresh_at: None,
            loading: false,
            daemon_connected: false,
            current_working_dir: PathBuf::new(),
        }
    }
}

impl SessionBrowserState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the browser and loads sessions from daemon + disk.
    pub fn open(&mut self, current_working_dir: &std::path::Path) {
        self.open = true;
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.error = None;
        self.resuming = false;
        self.confirmation_pending = None;
        self.current_working_dir = current_working_dir.to_path_buf();

        // Do initial synchronous load (will be async-refreshed later)
        self.refresh_sync(current_working_dir);
    }

    /// Synchronous refresh - loads from snapshots only (for initial open).
    /// Call refresh_async for full daemon + snapshot loading.
    fn refresh_sync(&mut self, current_working_dir: &std::path::Path) {
        self.loading = true;
        self.daemon_connected = false;

        let current_dir_canonical = std::fs::canonicalize(current_working_dir)
            .unwrap_or_else(|_| current_working_dir.to_path_buf());

        // Load snapshots from disk
        match list_snapshots(current_working_dir) {
            Ok(snapshots) => {
                self.entries = snapshots
                    .iter()
                    .map(|snapshot| {
                        let snapshot_dir_canonical = std::fs::canonicalize(&snapshot.working_dir)
                            .unwrap_or_else(|_| snapshot.working_dir.clone());
                        let is_current_dir = snapshot_dir_canonical == current_dir_canonical;
                        SessionEntry::from_snapshot(snapshot, is_current_dir)
                    })
                    .collect();
            }
            Err(e) => {
                self.error = Some(format!("Failed to load sessions: {}", e));
                self.entries.clear();
            }
        }

        self.sort_entries();
        self.loading = false;
        // Don't set last_refresh_at here - leave it None so should_auto_refresh()
        // triggers immediately to fetch live sessions from daemon.
        // last_refresh_at will be set by apply_refresh() after async completes.
    }

    /// Asynchronous refresh - loads from daemon AND snapshots.
    /// Returns the updated entries for the caller to apply.
    pub async fn refresh_async(
        current_working_dir: &std::path::Path,
    ) -> (Vec<SessionEntry>, bool, Option<String>) {
        let mut entries = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut daemon_connected = false;
        let mut error: Option<String> = None;

        let current_dir_canonical = std::fs::canonicalize(current_working_dir)
            .unwrap_or_else(|_| current_working_dir.to_path_buf());

        // First, load snapshot IDs to know which sessions have snapshots
        let snapshot_ids: HashSet<String> = match list_snapshots(current_working_dir) {
            Ok(snapshots) => snapshots
                .iter()
                .map(|s| s.workflow_session_id.clone())
                .collect(),
            Err(_) => HashSet::new(),
        };

        // Try to get live sessions from daemon
        let daemon_client = session_daemon::client::SessionDaemonClient::new(false);
        if daemon_client.is_connected() {
            daemon_connected = true;
            if let Ok(live_sessions) = daemon_client.list().await {
                for record in live_sessions {
                    let has_snapshot = snapshot_ids.contains(&record.workflow_session_id);
                    let is_current_dir = {
                        let record_dir_canonical = std::fs::canonicalize(&record.working_dir)
                            .unwrap_or_else(|_| record.working_dir.clone());
                        record_dir_canonical == current_dir_canonical
                    };

                    seen_ids.insert(record.workflow_session_id.clone());
                    entries.push(SessionEntry::from_live(
                        &record,
                        is_current_dir,
                        has_snapshot,
                    ));
                }
            }
        }

        // Load disk snapshots and merge (add ones not already in live list)
        match list_snapshots(current_working_dir) {
            Ok(snapshots) => {
                for snapshot in snapshots {
                    if !seen_ids.contains(&snapshot.workflow_session_id) {
                        let snapshot_dir_canonical = std::fs::canonicalize(&snapshot.working_dir)
                            .unwrap_or_else(|_| snapshot.working_dir.clone());
                        let is_current_dir = snapshot_dir_canonical == current_dir_canonical;
                        entries.push(SessionEntry::from_snapshot(&snapshot, is_current_dir));
                    }
                }
            }
            Err(e) => {
                if entries.is_empty() {
                    error = Some(format!("Failed to load sessions: {}", e));
                }
            }
        }

        // Sort: Running first, then Unresponsive, then Stopped, then by last_seen
        entries.sort_by(|a, b| {
            // First by liveness (Running > Unresponsive > Stopped)
            let liveness_order = |l: &LivenessState| match l {
                LivenessState::Running => 0,
                LivenessState::Unresponsive => 1,
                LivenessState::Stopped => 2,
            };
            let a_order = liveness_order(&a.liveness);
            let b_order = liveness_order(&b.liveness);

            match a_order.cmp(&b_order) {
                std::cmp::Ordering::Equal => {
                    // Then by last_seen (most recent first)
                    b.last_seen_at.cmp(&a.last_seen_at)
                }
                other => other,
            }
        });

        (entries, daemon_connected, error)
    }

    /// Apply async refresh results.
    pub fn apply_refresh(
        &mut self,
        entries: Vec<SessionEntry>,
        daemon_connected: bool,
        error: Option<String>,
    ) {
        self.entries = entries;
        self.daemon_connected = daemon_connected;
        self.error = error;
        self.loading = false;
        self.last_refresh_at = Some(Instant::now());

        // Ensure selection is still valid
        let filtered_len = self.filtered_entries().len();
        if self.selected_idx >= filtered_len && filtered_len > 0 {
            self.selected_idx = filtered_len - 1;
        }
        self.ensure_visible();
    }

    /// Sort entries by liveness and last_seen.
    fn sort_entries(&mut self) {
        self.entries.sort_by(|a, b| {
            let liveness_order = |l: &LivenessState| match l {
                LivenessState::Running => 0,
                LivenessState::Unresponsive => 1,
                LivenessState::Stopped => 2,
            };
            let a_order = liveness_order(&a.liveness);
            let b_order = liveness_order(&b.liveness);

            match a_order.cmp(&b_order) {
                std::cmp::Ordering::Equal => b.last_seen_at.cmp(&a.last_seen_at),
                other => other,
            }
        });
    }

    /// Closes the browser overlay.
    pub fn close(&mut self) {
        self.open = false;
        self.entries.clear();
        self.error = None;
        self.resuming = false;
        self.confirmation_pending = None;
        self.loading = false;
    }

    /// Returns the filtered list of entries based on current filter settings.
    pub fn filtered_entries(&self) -> Vec<&SessionEntry> {
        if self.filter_current_dir {
            self.entries.iter().filter(|e| e.is_current_dir).collect()
        } else {
            self.entries.iter().collect()
        }
    }

    /// Moves selection up.
    pub fn select_prev(&mut self) {
        let entries = self.filtered_entries();
        if entries.is_empty() {
            return;
        }
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        } else {
            self.selected_idx = entries.len() - 1;
        }
        self.ensure_visible();
    }

    /// Moves selection down.
    pub fn select_next(&mut self) {
        let entries = self.filtered_entries();
        if entries.is_empty() {
            return;
        }
        self.selected_idx = (self.selected_idx + 1) % entries.len();
        self.ensure_visible();
    }

    /// Toggles the current directory filter.
    pub fn toggle_filter(&mut self) {
        self.filter_current_dir = !self.filter_current_dir;
        // Reset selection if it would be out of bounds
        let entries = self.filtered_entries();
        if self.selected_idx >= entries.len() {
            self.selected_idx = entries.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
        self.ensure_visible();
    }

    /// Returns the currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&SessionEntry> {
        let entries = self.filtered_entries();
        entries.get(self.selected_idx).copied()
    }

    /// Check if initial async refresh is needed.
    /// Returns true only on first open (to fetch live sessions from daemon).
    /// Subsequent updates come via push notifications, not polling.
    pub fn should_auto_refresh(&self) -> bool {
        if !self.open || self.loading || self.confirmation_pending.is_some() {
            return false;
        }
        // Only trigger on initial open, not every 5 seconds
        // Push notifications handle all subsequent updates
        self.last_refresh_at.is_none()
    }

    /// Apply a session update from a daemon push notification.
    /// Updates an existing entry or inserts a new one.
    pub fn apply_session_update(&mut self, record: SessionRecord) {
        let current_dir_canonical = std::fs::canonicalize(&self.current_working_dir)
            .unwrap_or_else(|_| self.current_working_dir.clone());

        let record_dir_canonical = std::fs::canonicalize(&record.working_dir)
            .unwrap_or_else(|_| record.working_dir.clone());
        let is_current_dir = record_dir_canonical == current_dir_canonical;

        // Check if we have a snapshot for this session
        let has_snapshot = self
            .entries
            .iter()
            .find(|e| e.session_id == record.workflow_session_id)
            .map(|e| e.has_snapshot)
            .unwrap_or(false);

        // Find existing entry by session ID
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.session_id == record.workflow_session_id)
        {
            // Update existing entry
            entry.feature_name = record.feature_name.clone();
            entry.phase = record.phase.clone();
            entry.iteration = record.iteration;
            entry.workflow_status = record.workflow_status.clone();
            entry.liveness = record.liveness;
            entry.last_seen_at = record.last_heartbeat_at.clone();
            entry.last_seen_relative = format_relative_time(&record.last_heartbeat_at);
            entry.working_dir = record.working_dir.clone();
            entry.is_current_dir = is_current_dir;
            entry.is_resumable = entry.has_snapshot && record.liveness != LivenessState::Running;
            entry.pid = Some(record.pid);
            entry.is_live = true;
        } else {
            // Insert new entry
            let new_entry = SessionEntry::from_live(&record, is_current_dir, has_snapshot);
            self.entries.push(new_entry);
        }

        // Re-sort entries
        self.sort_entries();

        // Ensure selection is still valid
        let filtered_len = self.filtered_entries().len();
        if self.selected_idx >= filtered_len && filtered_len > 0 {
            self.selected_idx = filtered_len - 1;
        }
    }

    /// Start a confirmation dialog for force-stopping a session.
    pub fn start_force_stop_confirmation(&mut self, session_id: String) {
        self.confirmation_pending = Some(ConfirmationState::ForceStop { session_id });
    }

    /// Start a confirmation dialog for cross-directory resume.
    pub fn start_cross_directory_confirmation(&mut self, session_id: String, target_dir: PathBuf) {
        self.confirmation_pending = Some(ConfirmationState::CrossDirectoryResume {
            session_id,
            target_dir,
        });
    }

    /// Cancel the pending confirmation.
    pub fn cancel_confirmation(&mut self) {
        self.confirmation_pending = None;
    }

    /// Check if a confirmation is pending.
    #[allow(dead_code)]
    pub fn has_confirmation_pending(&self) -> bool {
        self.confirmation_pending.is_some()
    }

    /// Ensure the selected item is visible in the viewport.
    fn ensure_visible(&mut self) {
        const VIEWPORT_SIZE: usize = 10;

        if self.selected_idx < self.scroll_offset {
            self.scroll_offset = self.selected_idx;
        } else if self.selected_idx >= self.scroll_offset + VIEWPORT_SIZE {
            self.scroll_offset = self.selected_idx.saturating_sub(VIEWPORT_SIZE - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_browser_state_new() {
        let state = SessionBrowserState::new();
        assert!(!state.open);
        assert!(state.entries.is_empty());
        assert_eq!(state.selected_idx, 0);
        assert!(!state.daemon_connected);
        assert!(state.confirmation_pending.is_none());
    }

    #[test]
    fn test_session_browser_close() {
        let mut state = SessionBrowserState::new();
        state.open = true;
        state.confirmation_pending = Some(ConfirmationState::ForceStop {
            session_id: "test".to_string(),
        });
        state.entries.push(SessionEntry {
            session_id: "test".to_string(),
            feature_name: "test".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            workflow_status: "Planning".to_string(),
            liveness: LivenessState::Running,
            last_seen_at: "2024-01-01T00:00:00Z".to_string(),
            last_seen_relative: "just now".to_string(),
            working_dir: PathBuf::from("/test"),
            is_current_dir: true,
            has_snapshot: true,
            is_resumable: false,
            pid: Some(1234),
            is_live: true,
        });

        state.close();
        assert!(!state.open);
        assert!(state.entries.is_empty());
        assert!(state.confirmation_pending.is_none());
    }

    #[test]
    fn test_format_relative_time() {
        // Test "just now"
        let now = chrono::Utc::now().to_rfc3339();
        assert_eq!(format_relative_time(&now), "just now");

        // Test invalid timestamp
        assert_eq!(format_relative_time("invalid"), "unknown");
    }

    #[test]
    fn test_confirmation_states() {
        let mut state = SessionBrowserState::new();
        assert!(!state.has_confirmation_pending());

        state.start_force_stop_confirmation("test-session".to_string());
        assert!(state.has_confirmation_pending());
        assert!(matches!(
            state.confirmation_pending,
            Some(ConfirmationState::ForceStop { .. })
        ));

        state.cancel_confirmation();
        assert!(!state.has_confirmation_pending());

        state.start_cross_directory_confirmation(
            "test-session".to_string(),
            PathBuf::from("/other/dir"),
        );
        assert!(state.has_confirmation_pending());
        assert!(matches!(
            state.confirmation_pending,
            Some(ConfirmationState::CrossDirectoryResume { .. })
        ));
    }

    #[test]
    fn test_session_entry_resumability() {
        // Live Running session with snapshot - not resumable
        let entry = SessionEntry {
            session_id: "test".to_string(),
            feature_name: "test".to_string(),
            phase: "Planning".to_string(),
            iteration: 1,
            workflow_status: "Planning".to_string(),
            liveness: LivenessState::Running,
            last_seen_at: "2024-01-01T00:00:00Z".to_string(),
            last_seen_relative: "just now".to_string(),
            working_dir: PathBuf::from("/test"),
            is_current_dir: true,
            has_snapshot: true,
            is_resumable: false, // Running sessions aren't resumable
            pid: Some(1234),
            is_live: true,
        };
        assert!(!entry.is_resumable);

        // Stopped session with snapshot - resumable
        let entry2 = SessionEntry {
            liveness: LivenessState::Stopped,
            has_snapshot: true,
            is_resumable: true,
            ..entry.clone()
        };
        assert!(entry2.is_resumable);
    }

    fn create_test_record(id: &str, phase: &str, iteration: u32) -> SessionRecord {
        SessionRecord::new(
            id.to_string(),
            format!("{}-feature", id),
            PathBuf::from("/tmp/test"),
            PathBuf::from("/tmp/test/state.json"),
            phase.to_string(),
            iteration,
            phase.to_string(),
            std::process::id(),
        )
    }

    fn create_test_entry(id: &str, phase: &str, iteration: u32) -> SessionEntry {
        SessionEntry {
            session_id: id.to_string(),
            feature_name: format!("{}-feature", id),
            phase: phase.to_string(),
            iteration,
            workflow_status: phase.to_string(),
            liveness: LivenessState::Running,
            last_seen_at: "2024-01-01T00:00:00Z".to_string(),
            last_seen_relative: "just now".to_string(),
            working_dir: PathBuf::from("/tmp/test"),
            is_current_dir: false,
            has_snapshot: false,
            is_resumable: false,
            pid: Some(std::process::id()),
            is_live: true,
        }
    }

    #[test]
    fn test_apply_session_update_new_entry() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");
        assert!(state.entries.is_empty());

        // Apply update for new session
        let record = create_test_record("new-session", "Planning", 1);
        state.apply_session_update(record);

        // Should have added entry
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].session_id, "new-session");
        assert_eq!(state.entries[0].phase, "Planning");
        assert_eq!(state.entries[0].iteration, 1);
        assert!(state.entries[0].is_live);
    }

    #[test]
    fn test_apply_session_update_existing_entry() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");

        // Add initial entry
        state
            .entries
            .push(create_test_entry("existing-session", "Planning", 1));
        assert_eq!(state.entries.len(), 1);

        // Apply update - should update existing entry
        let mut record = create_test_record("existing-session", "Reviewing", 2);
        record.workflow_status = "Reviewing".to_string();
        state.apply_session_update(record);

        // Should still have 1 entry, but updated
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].session_id, "existing-session");
        assert_eq!(state.entries[0].phase, "Reviewing");
        assert_eq!(state.entries[0].iteration, 2);
    }

    #[test]
    fn test_apply_session_update_preserves_has_snapshot() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");

        // Add entry with snapshot
        let mut entry = create_test_entry("snapshot-session", "Planning", 1);
        entry.has_snapshot = true;
        state.entries.push(entry);

        // Apply update - should preserve has_snapshot
        let record = create_test_record("snapshot-session", "Reviewing", 1);
        state.apply_session_update(record);

        assert_eq!(state.entries.len(), 1);
        assert!(state.entries[0].has_snapshot);
    }

    #[test]
    fn test_apply_session_update_updates_liveness() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");

        // Add running entry
        state
            .entries
            .push(create_test_entry("live-session", "Planning", 1));

        // Create stopped record
        let mut record = create_test_record("live-session", "Planning", 1);
        record.liveness = LivenessState::Stopped;
        state.apply_session_update(record);

        assert_eq!(state.entries[0].liveness, LivenessState::Stopped);
    }

    #[test]
    fn test_apply_session_update_resumability() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");

        // Add entry with snapshot, running
        let mut entry = create_test_entry("resumable-session", "Planning", 1);
        entry.has_snapshot = true;
        entry.liveness = LivenessState::Running;
        entry.is_resumable = false; // Running = not resumable
        state.entries.push(entry);

        // When stopped, should become resumable
        let mut record = create_test_record("resumable-session", "Planning", 1);
        record.liveness = LivenessState::Stopped;
        state.apply_session_update(record);

        // has_snapshot=true + Stopped = resumable
        assert!(state.entries[0].is_resumable);
    }

    #[test]
    fn test_apply_session_update_multiple_sessions() {
        let mut state = SessionBrowserState::new();
        state.current_working_dir = PathBuf::from("/tmp/test");

        // Add two entries
        state
            .entries
            .push(create_test_entry("session-a", "Planning", 1));
        state
            .entries
            .push(create_test_entry("session-b", "Planning", 1));

        // Update only session-b
        let record = create_test_record("session-b", "Reviewing", 2);
        state.apply_session_update(record);

        // session-a unchanged, session-b updated
        let session_a = state
            .entries
            .iter()
            .find(|e| e.session_id == "session-a")
            .unwrap();
        let session_b = state
            .entries
            .iter()
            .find(|e| e.session_id == "session-b")
            .unwrap();

        assert_eq!(session_a.phase, "Planning");
        assert_eq!(session_a.iteration, 1);
        assert_eq!(session_b.phase, "Reviewing");
        assert_eq!(session_b.iteration, 2);
    }
}
