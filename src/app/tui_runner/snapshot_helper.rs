//! Snapshot creation helper for session persistence.
//!
//! This module provides a unified helper for creating and saving session snapshots,
//! used by both explicit stop handling and workflow completion.

use crate::planning_paths;
use crate::session_daemon::{save_snapshot, SessionSnapshot};
use crate::state::State;
use crate::tui::Session;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Creates and saves a session snapshot.
///
/// This is called when:
/// - A workflow is explicitly stopped by the user
/// - A workflow completes (success or error)
///
/// The `working_dir` parameter is used as the base working directory for the snapshot.
/// If the session has a context with a stored state_path, that path is used to ensure
/// consistency across resume cycles. Otherwise, the state_path is computed from working_dir.
///
/// Returns the path to the saved snapshot file on success.
pub fn create_and_save_snapshot(
    session: &Session,
    state: &State,
    working_dir: &Path,
) -> Result<PathBuf> {
    // Use state_path from session context if available (preserves original path across resumes)
    // Otherwise compute from working_dir (for new sessions)
    let state_path = session
        .context
        .as_ref()
        .map(|ctx| ctx.state_path.clone())
        .map(Ok)
        .unwrap_or_else(|| planning_paths::state_path(working_dir, &state.feature_name))?;

    // Get workflow name from session context (preserves the workflow used for this session)
    // If no context, fall back to current selection for this working directory
    let workflow_name = session
        .context
        .as_ref()
        .map(|ctx| ctx.workflow_config.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            crate::app::WorkflowSelection::load(working_dir)
                .map(|s| s.workflow)
                .unwrap_or_else(|_| "claude-only".to_string())
        });

    let ui_state = session.to_ui_state();
    let now = chrono::Utc::now().to_rfc3339();
    let mut state_copy = state.clone();
    state_copy.set_updated_at_with(&now);
    let elapsed = session.start_time.elapsed().as_millis() as u64;

    // Include workflow view and event sequence from the session
    let (workflow_view, last_event_sequence) = session
        .workflow_view
        .as_ref()
        .map(|v| (Some(v.clone()), v.last_event_sequence))
        .unwrap_or((None, 0));

    let snapshot = SessionSnapshot::new_with_timestamp(
        working_dir.to_path_buf(),
        state.workflow_session_id.clone(),
        state_path,
        state_copy,
        ui_state,
        elapsed,
        now,
        workflow_name,
        workflow_view,
        last_event_sequence,
    );

    save_snapshot(&snapshot)
}
