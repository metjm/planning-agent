//! Snapshot creation helper for session persistence.
//!
//! This module provides a unified helper for creating and saving session snapshots,
//! used by both explicit stop handling and workflow completion.

use crate::domain::view::WorkflowView;
use crate::planning_paths;
use crate::session_daemon::{save_snapshot, SessionSnapshot};
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
    view: &WorkflowView,
    working_dir: &Path,
) -> Result<PathBuf> {
    // Extract feature name from view (Option-wrapped)
    let feature_name = view
        .feature_name
        .as_ref()
        .map(|f| f.as_str())
        .unwrap_or("unknown");

    // Use state_path from session context if available (preserves original path across resumes)
    // Otherwise compute from working_dir (for new sessions)
    let state_path = session
        .context
        .as_ref()
        .map(|ctx| ctx.state_path.clone())
        .map(Ok)
        .unwrap_or_else(|| planning_paths::state_path(working_dir, feature_name))?;

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
    let elapsed = session.start_time.elapsed().as_millis() as u64;

    // Get workflow_session_id from view (WorkflowId -> String)
    let workflow_session_id = view
        .workflow_id
        .as_ref()
        .map(|id| id.to_string())
        .unwrap_or_default();

    // Include workflow view and event sequence
    let last_event_sequence = view.last_event_sequence;

    let snapshot = SessionSnapshot::new_with_timestamp(
        working_dir.to_path_buf(),
        workflow_session_id,
        state_path,
        ui_state,
        elapsed,
        now,
        workflow_name,
        view.clone(),
        last_event_sequence,
    );

    save_snapshot(&snapshot)
}
