use super::file_index::FileIndex;
use super::session::Session;
use super::session_browser::SessionBrowserState;
use crate::update::{UpdateStatus, VersionInfo};

pub struct TabManager {
    pub sessions: Vec<Session>,
    pub active_tab: usize,
    next_id: usize,

    pub update_status: UpdateStatus,

    pub update_in_progress: bool,

    pub update_error: Option<String>,

    pub update_spinner_frame: u8,

    pub update_notice: Option<String>,

    /// Shared file index for @-mention auto-complete across all sessions
    pub file_index: FileIndex,

    /// Notice from a slash command execution (success message)
    pub command_notice: Option<String>,

    /// Error from a slash command execution
    pub command_error: Option<String>,

    /// Whether a slash command is currently executing
    pub command_in_progress: bool,

    /// Cached version info for the current build (short SHA and commit date)
    pub version_info: Option<VersionInfo>,

    /// Session browser overlay state
    pub session_browser: SessionBrowserState,

    /// Whether the session daemon is connected (for footer status indicator)
    pub daemon_connected: bool,
}

/// TabManager provides the full API surface for multi-tab management.
/// Some methods may not be used in all code paths but are part of the public API.
impl TabManager {
    pub fn new() -> Self {
        let mut manager = Self {
            sessions: Vec::new(),
            active_tab: 0,
            next_id: 0,
            update_status: UpdateStatus::default(),
            update_in_progress: false,
            update_error: None,
            update_spinner_frame: 0,
            update_notice: None,
            file_index: FileIndex::new(),
            command_notice: None,
            command_error: None,
            command_in_progress: false,
            version_info: None,
            session_browser: SessionBrowserState::new(),
            daemon_connected: false,
        };

        manager.add_session();
        manager
    }

    pub fn add_session(&mut self) -> &mut Session {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session::new(id));
        let idx = self.sessions.len() - 1;
        self.active_tab = idx;
        &mut self.sessions[idx]
    }

    pub fn add_session_with_name(&mut self, name: String) -> &mut Session {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session::with_name(id, name));
        let idx = self.sessions.len() - 1;
        self.active_tab = idx;
        &mut self.sessions[idx]
    }

    pub fn close_tab(&mut self, index: usize) {
        if self.sessions.len() <= 1 {
            return;
        }

        if index >= self.sessions.len() {
            return;
        }

        let session = self.sessions.remove(index);

        drop(session.approval_tx);

        if self.active_tab >= self.sessions.len() {
            self.active_tab = self.sessions.len() - 1;
        } else if self.active_tab > index {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
    }

    pub fn close_current_if_empty(&mut self) {
        let is_empty = {
            let session = &self.sessions[self.active_tab];
            session.name.is_empty() && session.output_lines.is_empty()
        };
        if is_empty {
            self.close_tab(self.active_tab);
        }
    }

    pub fn active(&self) -> &Session {
        &self.sessions[self.active_tab]
    }

    pub fn active_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active_tab]
    }

    pub fn next_tab(&mut self) {
        if self.sessions.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.sessions.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if self.sessions.len() > 1 {
            self.active_tab = if self.active_tab == 0 {
                self.sessions.len() - 1
            } else {
                self.active_tab - 1
            };
        }
    }

    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active_tab = index;
        }
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn session_by_id_mut(&mut self, id: usize) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    pub fn sessions_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
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

        manager.active_tab = 2;
        manager.close_tab(1);
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active_tab, 1);
    }

    #[test]
    fn test_cannot_close_last_tab() {
        let mut manager = TabManager::new();
        assert_eq!(manager.len(), 1);

        manager.close_tab(0);
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_add_session_with_name() {
        let mut manager = TabManager::new();
        manager.add_session_with_name("test-feature".to_string());

        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active().name, "test-feature");
    }
}
