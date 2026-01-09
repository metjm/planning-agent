use crossterm::{execute, terminal::SetTitle};
use std::io::{self, IsTerminal, Write};

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

    pub fn save_title(&self) {
        if self.is_supported {

            let _ = io::stdout().write_all(b"\x1b[22;0t");
            let _ = io::stdout().flush();
        }
    }

    pub fn restore_title(&self) {
        if self.is_supported {

            let _ = io::stdout().write_all(b"\x1b[23;0t");
            let _ = io::stdout().flush();
        }
    }

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
        let _ = manager.is_supported;
    }
}
