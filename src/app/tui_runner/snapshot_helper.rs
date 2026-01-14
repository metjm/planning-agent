//! Snapshot creation helper for session persistence.
//!
//! This module provides a unified helper for creating and saving session snapshots,
//! used by both explicit stop handling and workflow completion.

use crate::planning_paths;
use crate::session_store::{save_snapshot, SessionSnapshot};
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
/// Returns the path to the saved snapshot file on success.
pub fn create_and_save_snapshot(
    session: &Session,
    state: &State,
    working_dir: &Path,
) -> Result<PathBuf> {
    let state_path = planning_paths::state_path(working_dir, &state.feature_name)?;
    let ui_state = session.to_ui_state();
    let now = chrono::Utc::now().to_rfc3339();
    let mut state_copy = state.clone();
    state_copy.set_updated_at_with(&now);
    let elapsed = session.start_time.elapsed().as_millis() as u64;

    let snapshot = SessionSnapshot::new_with_timestamp(
        working_dir.to_path_buf(),
        state.workflow_session_id.clone(),
        state_path,
        state_copy,
        ui_state,
        elapsed,
        now,
    );

    save_snapshot(working_dir, &snapshot)
}
