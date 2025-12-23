use super::session::Session;
use crate::update::UpdateStatus;

/// Manages multiple session tabs
#[allow(dead_code)]
pub struct TabManager {
    pub sessions: Vec<Session>,
    pub active_tab: usize,
    next_id: usize,
    /// Global update status (shared across all tabs)
    pub update_status: UpdateStatus,
    /// Whether an update is currently being installed
    pub update_in_progress: bool,
    /// Error message from failed update attempt
    pub update_error: Option<String>,
}

#[allow(dead_code)]
impl TabManager {
    pub fn new() -> Self {
        let mut manager = Self {
            sessions: Vec::new(),
            active_tab: 0,
            next_id: 0,
            update_status: UpdateStatus::default(),
            update_in_progress: false,
            update_error: None,
        };
        // Start with one session
        manager.add_session();
        manager
    }

    /// Add a new session and return a mutable reference to it
    pub fn add_session(&mut self) -> &mut Session {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session::new(id));
        let idx = self.sessions.len() - 1;
        self.active_tab = idx;
        &mut self.sessions[idx]
    }

    /// Add a session with a pre-set name (for CLI-provided objectives)
    pub fn add_session_with_name(&mut self, name: String) -> &mut Session {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session::with_name(id, name));
        let idx = self.sessions.len() - 1;
        self.active_tab = idx;
        &mut self.sessions[idx]
    }

    /// Close a session at the given index
    pub fn close_tab(&mut self, index: usize) {
        if self.sessions.len() <= 1 {
            return; // Keep at least one tab
        }

        if index >= self.sessions.len() {
            return;
        }

        let session = self.sessions.remove(index);

        // Graceful workflow cancellation via channel closure
        // Dropping approval_tx signals the workflow to terminate gracefully
        drop(session.approval_tx);
        // The workflow handle will be dropped, which doesn't abort the task
        // but the task should detect channel closure and terminate

        // Adjust active tab index
        if self.active_tab >= self.sessions.len() {
            self.active_tab = self.sessions.len() - 1;
        } else if self.active_tab > index {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
    }

    /// Close the current tab if it's empty
    pub fn close_current_if_empty(&mut self) {
        let is_empty = {
            let session = &self.sessions[self.active_tab];
            session.name.is_empty() && session.output_lines.is_empty()
        };
        if is_empty {
            self.close_tab(self.active_tab);
        }
    }

    /// Get the active session
    pub fn active(&self) -> &Session {
        &self.sessions[self.active_tab]
    }

    /// Get the active session mutably
    pub fn active_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active_tab]
    }

    /// Switch to the next tab (wraps around)
    pub fn next_tab(&mut self) {
        if self.sessions.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.sessions.len();
        }
    }

    /// Switch to the previous tab (wraps around)
    pub fn prev_tab(&mut self) {
        if self.sessions.len() > 1 {
            self.active_tab = if self.active_tab == 0 {
                self.sessions.len() - 1
            } else {
                self.active_tab - 1
            };
        }
    }

    /// Switch to a specific tab by index (0-indexed)
    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active_tab = index;
        }
    }

    /// Get all tab titles for display
    pub fn tab_titles(&self) -> Vec<&str> {
        self.sessions
            .iter()
            .map(|s| {
                if s.name.is_empty() {
                    "New Tab"
                } else {
                    s.name.as_str()
                }
            })
            .collect()
    }

    /// Get the number of sessions
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Check if there are no sessions
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Find a session by ID and return a mutable reference
    pub fn session_by_id_mut(&mut self, id: usize) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Find a session by ID and return a reference
    pub fn session_by_id(&self, id: usize) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Get an iterator over all sessions mutably
    pub fn sessions_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }

    /// Switch to the session with the given ID
    pub fn switch_to_session(&mut self, session_id: usize) {
        if let Some(idx) = self.sessions.iter().position(|s| s.id == session_id) {
            self.active_tab = idx;
        }
    }

    /// Check if any session needs attention (has pending approval)
    pub fn has_pending_approval(&self) -> bool {
        self.sessions.iter().any(|s| {
            s.approval_mode != super::session::ApprovalMode::None
        })
    }

    /// Get indices of sessions needing attention
    pub fn sessions_needing_attention(&self) -> Vec<usize> {
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.approval_mode != super::session::ApprovalMode::None)
            .map(|(i, _)| i)
            .collect()
    }
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_one_session() {
        let manager = TabManager::new();
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.active_tab, 0);
    }

    #[test]
    fn test_add_session() {
        let mut manager = TabManager::new();
        let initial_len = manager.len();

        manager.add_session();
        assert_eq!(manager.len(), initial_len + 1);
        assert_eq!(manager.active_tab, manager.len() - 1);
    }

    #[test]
    fn test_next_tab_wraps() {
        let mut manager = TabManager::new();
        manager.add_session();
        manager.add_session();
        assert_eq!(manager.len(), 3);

        manager.active_tab = 2;
        manager.next_tab();
        assert_eq!(manager.active_tab, 0);
    }

    #[test]
    fn test_prev_tab_wraps() {
        let mut manager = TabManager::new();
        manager.add_session();
        manager.add_session();
        assert_eq!(manager.len(), 3);

        manager.active_tab = 0;
        manager.prev_tab();
        assert_eq!(manager.active_tab, 2);
    }

    #[test]
    fn test_switch_to_tab() {
        let mut manager = TabManager::new();
        manager.add_session();
        manager.add_session();

        manager.switch_to_tab(1);
        assert_eq!(manager.active_tab, 1);

        // Out of bounds should be ignored
        manager.switch_to_tab(100);
        assert_eq!(manager.active_tab, 1);
    }

    #[test]
    fn test_switch_to_tab_out_of_bounds() {
        let mut manager = TabManager::new();
        manager.switch_to_tab(100);
        assert_eq!(manager.active_tab, 0);
    }

    #[test]
    fn test_close_tab_adjusts_active() {
        let mut manager = TabManager::new();
        manager.add_session();
        manager.add_session();
        assert_eq!(manager.len(), 3);

        // Active is at 2, close tab 1
        manager.active_tab = 2;
        manager.close_tab(1);
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active_tab, 1); // Adjusted down by 1
    }

    #[test]
    fn test_cannot_close_last_tab() {
        let mut manager = TabManager::new();
        assert_eq!(manager.len(), 1);

        manager.close_tab(0);
        assert_eq!(manager.len(), 1); // Still has one tab
    }

    #[test]
    fn test_session_by_id() {
        let mut manager = TabManager::new();
        let first_id = manager.sessions[0].id;

        manager.add_session();
        let second_id = manager.sessions[1].id;

        assert!(manager.session_by_id(first_id).is_some());
        assert!(manager.session_by_id(second_id).is_some());
        assert!(manager.session_by_id(999).is_none());
    }

    #[test]
    fn test_session_by_id_after_close() {
        let mut manager = TabManager::new();
        manager.add_session();

        let first_id = manager.sessions[0].id;
        let second_id = manager.sessions[1].id;

        manager.close_tab(0);

        assert!(manager.session_by_id(first_id).is_none());
        assert!(manager.session_by_id(second_id).is_some());
    }

    #[test]
    fn test_add_session_with_name() {
        let mut manager = TabManager::new();
        manager.add_session_with_name("test-feature".to_string());

        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active().name, "test-feature");
    }
}
