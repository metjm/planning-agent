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

    /// Saves the current terminal title to the terminal's title stack.
    /// Uses `let _ =` because terminal title operations are cosmetic; if stdout
    /// is unavailable (pipe closed, terminal gone), there's nothing useful to do.
    pub fn save_title(&self) {
        if self.is_supported {
            let _ = io::stdout().write_all(b"\x1b[22;0t");
            let _ = io::stdout().flush();
        }
    }

    /// Restores the previously saved terminal title from the terminal's title stack.
    /// Uses `let _ =` because terminal title operations are cosmetic; if stdout
    /// is unavailable (pipe closed, terminal gone), there's nothing useful to do.
    pub fn restore_title(&self) {
        if self.is_supported {
            let _ = io::stdout().write_all(b"\x1b[23;0t");
            let _ = io::stdout().flush();
        }
    }

    /// Sets the terminal title to the given string.
    /// Uses `let _ =` because terminal title operations are cosmetic; if stdout
    /// is unavailable (pipe closed, terminal gone), there's nothing useful to do.
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
