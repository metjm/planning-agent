//! CLI instance tracking methods for Session.
//!
//! This module provides CLI agent process lifecycle tracking (started, activity, finished)
//! for the TUI session. It tracks active CLI processes with elapsed runtime and idle time.

use super::Session;
use std::time::Instant;

/// Unique identifier for a CLI instance within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CliInstanceId(pub u64);

/// Represents an active CLI agent process.
#[derive(Debug, Clone)]
pub struct CliInstance {
    /// Unique identifier for this instance within the session.
    pub id: CliInstanceId,
    /// Name of the agent (e.g., "claude", "codex", "gemini").
    pub agent_name: String,
    /// Process ID if available (from Child::id()).
    pub pid: Option<u32>,
    /// When the process was started.
    pub started_at: Instant,
    /// When the process last produced output.
    pub last_activity_at: Instant,
}

impl CliInstance {
    /// Creates a new CLI instance.
    pub fn new(
        id: CliInstanceId,
        agent_name: String,
        pid: Option<u32>,
        started_at: Instant,
    ) -> Self {
        Self {
            id,
            agent_name,
            pid,
            started_at,
            last_activity_at: started_at,
        }
    }

    /// Returns the elapsed time since the process started.
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }

    /// Returns the idle time since last activity.
    pub fn idle(&self) -> std::time::Duration {
        self.last_activity_at.elapsed()
    }

    /// Returns a display label for this instance.
    /// Uses pid if available, otherwise falls back to #<id>.
    pub fn display_label(&self) -> String {
        if let Some(pid) = self.pid {
            format!("{} (pid {})", self.agent_name, pid)
        } else {
            format!("{} (#{})", self.agent_name, self.id.0)
        }
    }
}

impl Session {
    /// Record that a CLI instance has started.
    pub fn cli_instance_started(
        &mut self,
        id: CliInstanceId,
        agent_name: String,
        pid: Option<u32>,
        started_at: Instant,
    ) {
        let instance = CliInstance::new(id, agent_name, pid, started_at);
        self.cli_instances.push(instance);
    }

    /// Record activity for a CLI instance (updates last_activity_at).
    pub fn cli_instance_activity(&mut self, id: CliInstanceId, activity_at: Instant) {
        if let Some(instance) = self.cli_instances.iter_mut().find(|i| i.id == id) {
            instance.last_activity_at = activity_at;
        }
    }

    /// Record that a CLI instance has finished and remove it.
    pub fn cli_instance_finished(&mut self, id: CliInstanceId) {
        self.cli_instances.retain(|i| i.id != id);
    }

    /// Get all active CLI instances sorted by started_at (oldest first).
    pub fn cli_instances_sorted(&self) -> Vec<&CliInstance> {
        let mut instances: Vec<_> = self.cli_instances.iter().collect();
        instances.sort_by_key(|i| i.started_at);
        instances
    }
}
