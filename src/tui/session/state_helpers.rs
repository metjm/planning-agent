//! Session state helper methods.
//!
//! This module provides convenience methods for accessing state information
//! that can come from either the new state_snapshot (watch channel) or
//! legacy workflow_state, providing a smooth migration path.

use super::Session;
use crate::state::Phase;

#[allow(dead_code)] // Methods will be used when migration is complete
impl Session {
    /// Polls the snapshot receiver for state updates.
    /// Called from TUI event loop after processing events.
    /// Non-blocking - only updates if new value available.
    pub fn poll_state_updates(&mut self) {
        if let Some(rx) = &mut self.snapshot_rx {
            match rx.has_changed() {
                Ok(true) => {
                    let snapshot = rx.borrow_and_update().clone();
                    // Also update session name from feature_name
                    self.name = snapshot.feature_name.clone();
                    self.state_snapshot = Some(snapshot);
                }
                Ok(false) => {} // No change
                Err(_) => {
                    // Sender dropped - workflow task ended or panicked
                    self.snapshot_rx = None;
                }
            }
        }
    }

    /// Returns the current phase from snapshot, or from legacy workflow_state.
    pub fn current_phase(&self) -> Option<&Phase> {
        self.state_snapshot
            .as_ref()
            .map(|s| &s.phase)
            .or_else(|| self.workflow_state.as_ref().map(|s| &s.phase))
    }

    /// Returns the objective from snapshot, or from legacy workflow_state.
    pub fn objective(&self) -> &str {
        self.state_snapshot
            .as_ref()
            .map(|s| s.objective.as_str())
            .or_else(|| self.workflow_state.as_ref().map(|s| s.objective.as_str()))
            .unwrap_or("")
    }

    /// Returns the plan file path from snapshot, or from legacy workflow_state.
    pub fn plan_file(&self) -> Option<&std::path::Path> {
        self.state_snapshot
            .as_ref()
            .map(|s| s.plan_file.as_path())
            .or_else(|| self.workflow_state.as_ref().map(|s| s.plan_file.as_path()))
    }

    /// Returns the current iteration from snapshot, or from legacy workflow_state.
    pub fn current_iteration(&self) -> u32 {
        self.state_snapshot
            .as_ref()
            .map(|s| s.iteration)
            .or_else(|| self.workflow_state.as_ref().map(|s| s.iteration))
            .unwrap_or(1)
    }

    /// Returns the max iterations from snapshot, or from legacy workflow_state.
    pub fn max_iterations(&self) -> u32 {
        self.state_snapshot
            .as_ref()
            .map(|s| s.max_iterations)
            .or_else(|| self.workflow_state.as_ref().map(|s| s.max_iterations))
            .unwrap_or(3)
    }

    /// Returns the feature name from snapshot, or from legacy workflow_state.
    pub fn feature_name(&self) -> &str {
        self.state_snapshot
            .as_ref()
            .map(|s| s.feature_name.as_str())
            .or_else(|| {
                self.workflow_state
                    .as_ref()
                    .map(|s| s.feature_name.as_str())
            })
            .unwrap_or(&self.name)
    }

    /// Returns the workflow session ID from snapshot, or from legacy workflow_state.
    pub fn workflow_session_id(&self) -> Option<&str> {
        self.state_snapshot
            .as_ref()
            .map(|s| s.workflow_session_id.as_str())
            .or_else(|| {
                self.workflow_state
                    .as_ref()
                    .map(|s| s.workflow_session_id.as_str())
            })
    }
}
