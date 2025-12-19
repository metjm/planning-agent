use crossterm::{execute, terminal::SetTitle};
use std::io::{self, IsTerminal, Write};

/// Manages terminal window title with automatic restoration
pub struct TerminalTitleManager {
    is_supported: bool,
}

impl TerminalTitleManager {
    pub fn new() -> Self {
        let is_supported = std::io::stdout().is_terminal()
            && std::env::var("CI").is_err()
            && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);

        Self { is_supported }
    }

    /// Save current title to xterm stack (call once at startup)
    pub fn save_title(&self) {
        if self.is_supported {
            // CSI 22 ; 0 t - Save icon and window title on stack
            let _ = io::stdout().write_all(b"\x1b[22;0t");
            let _ = io::stdout().flush();
        }
    }

    /// Restore title from xterm stack (call on exit)
    pub fn restore_title(&self) {
        if self.is_supported {
            // CSI 23 ; 0 t - Restore icon and window title from stack
            let _ = io::stdout().write_all(b"\x1b[23;0t");
            let _ = io::stdout().flush();
        }
    }

    /// Set terminal title
    pub fn set_title(&self, title: &str) {
        if self.is_supported {
            let _ = execute!(io::stdout(), SetTitle(title));
        }
    }
}

impl Default for TerminalTitleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_manager_creation() {
        let manager = TerminalTitleManager::new();
        // Just verify it doesn't panic
        assert!(true);
        // is_supported will likely be false in test environment
        let _ = manager.is_supported;
    }
}
