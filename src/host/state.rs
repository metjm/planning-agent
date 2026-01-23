//! Host state management.
//!
//! Tracks connected containers and their sessions, providing aggregated
//! session data for display in the GUI.

use crate::host_protocol::SessionInfo;
use std::collections::HashMap;
use std::time::Instant;

/// Represents a connected container daemon.
#[derive(Debug, Clone)]
pub struct ConnectedContainer {
    pub container_name: String,
    pub last_message_at: Instant,
    pub sessions: HashMap<String, SessionInfo>,
    /// Git commit SHA the daemon was built from.
    pub git_sha: String,
    /// Unix timestamp when the daemon was built.
    pub build_timestamp: u64,
}

/// Aggregated state for the host application.
pub struct HostState {
    /// Connected containers by container_id
    pub containers: HashMap<String, ConnectedContainer>,
    /// Flattened session list for display (computed on demand)
    cached_sessions: Option<Vec<DisplaySession>>,
    /// Last time any session changed (for "last update" display)
    pub last_update: Instant,
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

/// Session with container context for display.
#[derive(Debug, Clone)]
pub struct DisplaySession {
    pub container_name: String,
    pub session: SessionInfo,
}

impl HostState {
    pub fn new() -> Self {
        Self {
            containers: HashMap::new(),
            cached_sessions: None,
            last_update: Instant::now(),
        }
    }

    /// Register a new container connection.
    pub fn add_container(
        &mut self,
        container_id: String,
        container_name: String,
        git_sha: String,
        build_timestamp: u64,
    ) {
        let now = Instant::now();
        self.containers.insert(
            container_id,
            ConnectedContainer {
                container_name,
                last_message_at: now,
                sessions: HashMap::new(),
                git_sha,
                build_timestamp,
            },
        );
        self.invalidate_cache();
    }

    /// Remove a container (on disconnect).
    pub fn remove_container(&mut self, container_id: &str) {
        self.containers.remove(container_id);
        self.invalidate_cache();
    }

    /// Sync all sessions for a container.
    pub fn sync_sessions(&mut self, container_id: &str, sessions: Vec<SessionInfo>) {
        if let Some(container) = self.containers.get_mut(container_id) {
            eprintln!(
                "[host-state] sync_sessions: {} sessions for container '{}'",
                sessions.len(),
                container_id
            );
            container.sessions.clear();
            for session in &sessions {
                eprintln!(
                    "[host-state]   - {} (feature: {})",
                    session.session_id, session.feature_name
                );
            }
            for session in sessions {
                container
                    .sessions
                    .insert(session.session_id.clone(), session);
            }
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        } else {
            eprintln!(
                "[host-state] WARNING: sync_sessions called for unknown container '{}'",
                container_id
            );
        }
    }

    /// Update a single session.
    pub fn update_session(&mut self, container_id: &str, session: SessionInfo) {
        if let Some(container) = self.containers.get_mut(container_id) {
            eprintln!(
                "[host-state] update_session: {} (feature: {}) in container '{}'",
                session.session_id, session.feature_name, container_id
            );
            container
                .sessions
                .insert(session.session_id.clone(), session);
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        } else {
            eprintln!(
                "[host-state] WARNING: update_session called for unknown container '{}'",
                container_id
            );
        }
    }

    /// Remove a session.
    pub fn remove_session(&mut self, container_id: &str, session_id: &str) {
        if let Some(container) = self.containers.get_mut(container_id) {
            container.sessions.remove(session_id);
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        }
    }

    /// Record heartbeat from container.
    pub fn heartbeat(&mut self, container_id: &str) {
        if let Some(container) = self.containers.get_mut(container_id) {
            container.last_message_at = Instant::now();
        }
    }

    /// Get flattened session list for display.
    pub fn sessions(&mut self) -> &[DisplaySession] {
        if self.cached_sessions.is_none() {
            let mut sessions = Vec::new();
            for container in self.containers.values() {
                for session in container.sessions.values() {
                    sessions.push(DisplaySession {
                        container_name: container.container_name.clone(),
                        session: session.clone(),
                    });
                }
            }
            // Sort: AwaitingApproval first, then by updated_at descending
            sessions.sort_by(|a, b| {
                let status_order = |s: &str| match s.to_lowercase().as_str() {
                    "awaitingapproval" | "awaiting_approval" => 0,
                    "running" | "planning" | "reviewing" | "revising" => 1,
                    "error" => 2,
                    "stopped" => 3,
                    "complete" => 4,
                    _ => 5,
                };
                let a_order = status_order(&a.session.status);
                let b_order = status_order(&b.session.status);
                match a_order.cmp(&b_order) {
                    std::cmp::Ordering::Equal => b.session.updated_at.cmp(&a.session.updated_at),
                    other => other,
                }
            });
            self.cached_sessions = Some(sessions);
        }
        self.cached_sessions.as_ref().unwrap()
    }

    /// Count of sessions awaiting approval.
    pub fn approval_count(&self) -> usize {
        self.containers
            .values()
            .flat_map(|c| c.sessions.values())
            .filter(|s| s.status.to_lowercase().contains("approval"))
            .count()
    }

    /// Count of active (non-complete) sessions.
    pub fn active_count(&self) -> usize {
        self.containers
            .values()
            .flat_map(|c| c.sessions.values())
            .filter(|s| s.status.to_lowercase() != "complete")
            .count()
    }

    fn invalidate_cache(&mut self) {
        self.cached_sessions = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_daemon::LivenessState;

    fn make_session(id: &str, status: &str) -> SessionInfo {
        SessionInfo {
            session_id: id.to_string(),
            feature_name: format!("feature-{}", id),
            phase: "Planning".to_string(),
            iteration: 1,
            status: status.to_string(),
            liveness: LivenessState::Running,
            started_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_add_container() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );

        assert_eq!(state.containers.len(), 1);
        assert!(state.containers.contains_key("c1"));

        // Verify build info is stored
        let container = state.containers.get("c1").unwrap();
        assert_eq!(container.git_sha, "abc123");
        assert_eq!(container.build_timestamp, 1234567890);
    }

    #[test]
    fn test_sync_sessions() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );

        let sessions = vec![
            make_session("s1", "Running"),
            make_session("s2", "AwaitingApproval"),
        ];
        state.sync_sessions("c1", sessions);

        assert_eq!(state.active_count(), 2);
        assert_eq!(state.approval_count(), 1);
    }

    #[test]
    fn test_sessions_sorted_by_status() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );

        let sessions = vec![
            make_session("s1", "Running"),
            make_session("s2", "AwaitingApproval"),
            make_session("s3", "Complete"),
        ];
        state.sync_sessions("c1", sessions);

        let display = state.sessions();
        assert_eq!(display.len(), 3);
        // AwaitingApproval should be first
        assert!(display[0]
            .session
            .status
            .to_lowercase()
            .contains("approval"));
        // Complete should be last
        assert_eq!(display[2].session.status.to_lowercase(), "complete");
    }

    #[test]
    fn test_remove_container() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );
        state.sync_sessions("c1", vec![make_session("s1", "Running")]);

        state.remove_container("c1");

        assert_eq!(state.containers.len(), 0);
        assert_eq!(state.active_count(), 0);
    }

    #[test]
    fn test_update_session() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );
        state.sync_sessions("c1", vec![make_session("s1", "Running")]);

        // Update the session
        let updated = make_session("s1", "AwaitingApproval");
        state.update_session("c1", updated);

        assert_eq!(state.approval_count(), 1);
    }

    #[test]
    fn test_remove_session() {
        let mut state = HostState::new();
        state.add_container(
            "c1".to_string(),
            "Container 1".to_string(),
            "abc123".to_string(),
            1234567890,
        );
        state.sync_sessions(
            "c1",
            vec![make_session("s1", "Running"), make_session("s2", "Running")],
        );

        state.remove_session("c1", "s1");

        assert_eq!(state.active_count(), 1);
    }
}
