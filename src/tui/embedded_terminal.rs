//! Embedded terminal implementation for running Claude Code inside the TUI.
//!
//! This module provides PTY-backed terminal emulation using `portable-pty` for process
//! management and `vt100` for terminal state/rendering.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::tui::Event;

/// Default scrollback buffer size (number of lines)
pub const DEFAULT_SCROLLBACK_LEN: usize = 10_000;

/// Minimum terminal size required for implementation mode
pub const MIN_TERMINAL_COLS: u16 = 80;
pub const MIN_TERMINAL_ROWS: u16 = 24;

/// State for the embedded implementation terminal
pub struct EmbeddedTerminal {
    /// Terminal emulator (vt100) for parsing ANSI sequences
    parser: vt100::Parser,
    /// PTY master for resize operations
    pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    /// PTY writer for sending input to the child process
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Child process killer for cleanup
    child_killer: Box<dyn ChildKiller + Send + Sync>,
    /// Current scroll offset (0 = bottom/latest)
    pub scroll_offset: usize,
    /// Whether to auto-scroll to bottom on new output
    pub follow_mode: bool,
    /// Last known terminal size
    pub last_size: (u16, u16),
    /// Whether the terminal is currently active
    pub active: bool,
    /// Reader thread handle for cleanup
    reader_handle: Option<std::thread::JoinHandle<()>>,
    /// Flag to signal reader thread to stop
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
    /// Exit code when process completes (None if still running)
    pub exit_code: Option<i32>,
}

impl EmbeddedTerminal {
    /// Spawns a new embedded terminal running Claude CLI.
    ///
    /// # Arguments
    /// * `plan_path` - Path to the plan file to implement
    /// * `working_dir` - Workspace root directory for the implementation
    /// * `rows` - Initial terminal height
    /// * `cols` - Initial terminal width
    /// * `session_id` - Session ID for event routing
    /// * `event_tx` - Channel to send terminal output events
    pub fn spawn(
        plan_path: &Path,
        working_dir: &Path,
        rows: u16,
        cols: u16,
        session_id: usize,
        event_tx: mpsc::UnboundedSender<Event>,
    ) -> Result<Self> {
        // Validate minimum terminal size
        if cols < MIN_TERMINAL_COLS || rows < MIN_TERMINAL_ROWS {
            anyhow::bail!(
                "Terminal too small for implementation mode. Minimum size: {}x{}, current: {}x{}",
                MIN_TERMINAL_COLS,
                MIN_TERMINAL_ROWS,
                cols,
                rows
            );
        }

        // Create PTY system
        let pty_system = native_pty_system();

        // Open PTY with the specified size
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        // Get the writer BEFORE spawning the child
        let writer = pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;

        // Build the command with workspace root and absolute path instruction
        let prompt = format!(
            "Workspace Root: {}\n\nPlease implement the following plan fully: {}\n\nIMPORTANT: Use absolute paths for all file operations. Work within the workspace root.",
            working_dir.display(),
            plan_path.display()
        );

        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--dangerously-skip-permissions");
        cmd.arg(&prompt);

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn Claude CLI. Is it installed?")?;

        // Get the reader for streaming output
        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;

        // Create vt100 parser with scrollback
        let parser = vt100::Parser::new(rows, cols, DEFAULT_SCROLLBACK_LEN);

        // Create stop flag for reader thread
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);

        // Spawn reader thread to stream PTY output
        // Using blocking reads - the thread will exit when the PTY EOF occurs
        let reader_handle = std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];

            loop {
                // Check stop flag before blocking read
                if stop_flag_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) => {
                        // EOF - process exited
                        let _ = event_tx.send(Event::ImplementationExited {
                            session_id,
                            exit_code: None,
                        });
                        break;
                    }
                    Ok(n) => {
                        let chunk = buf[..n].to_vec();
                        let _ = event_tx.send(Event::ImplementationOutput {
                            session_id,
                            chunk,
                        });
                    }
                    Err(e) => {
                        // Interrupted or would block - check stop flag and continue
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        if e.kind() != std::io::ErrorKind::WouldBlock {
                            let _ = event_tx.send(Event::ImplementationError {
                                session_id,
                                error: format!("PTY read error: {}", e),
                            });
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            parser,
            pty_master: Arc::new(Mutex::new(pair.master)),
            pty_writer: Arc::new(Mutex::new(writer)),
            child_killer: child.clone_killer(),
            scroll_offset: 0,
            follow_mode: true,
            last_size: (rows, cols),
            active: true,
            reader_handle: Some(reader_handle),
            stop_flag,
            exit_code: None,
        })
    }

    /// Process output bytes from the PTY into the terminal emulator
    pub fn process_output(&mut self, chunk: &[u8]) {
        self.parser.process(chunk);

        // Auto-scroll to bottom if follow mode is enabled
        if self.follow_mode {
            self.scroll_offset = 0;
        }
    }

    /// Send a byte sequence to the PTY (for keyboard input)
    pub fn send_input(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.pty_writer.lock().map_err(|_| anyhow::anyhow!("PTY writer lock poisoned"))?;
        writer.write_all(bytes).context("Failed to write to PTY")?;
        writer.flush().context("Failed to flush PTY")?;
        Ok(())
    }

    /// Resize the PTY and terminal emulator
    /// Enforces minimum size to prevent issues with small terminals
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if rows < 1 || cols < 1 {
            return Ok(());
        }

        // Enforce minimum size - clamp to minimum values
        // This prevents issues when terminal becomes very small
        let rows = rows.max(MIN_TERMINAL_ROWS);
        let cols = cols.max(MIN_TERMINAL_COLS);

        // Skip resize if size hasn't changed
        if (rows, cols) == self.last_size {
            return Ok(());
        }

        // Resize PTY
        if let Ok(master) = self.pty_master.lock() {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .context("Failed to resize PTY")?;
        }

        // Resize vt100 screen
        self.parser.screen_mut().set_size(rows, cols);
        self.last_size = (rows, cols);

        Ok(())
    }

    /// Get the vt100 screen for rendering
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Scroll up by one line
    pub fn scroll_up(&mut self) {
        self.follow_mode = false;
        let max_scroll = self.max_scroll();
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
        }
    }

    /// Scroll down by one line
    pub fn scroll_down(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
        if self.scroll_offset == 0 {
            self.follow_mode = true;
        }
    }

    /// Scroll to top of buffer
    pub fn scroll_to_top(&mut self) {
        self.follow_mode = false;
        self.scroll_offset = self.max_scroll();
    }

    /// Scroll to bottom
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.follow_mode = true;
    }

    /// Get maximum scroll offset
    fn max_scroll(&self) -> usize {
        // vt100 stores scrollback, get total content rows
        let screen = self.parser.screen();
        let visible_rows = screen.size().0 as usize;
        // scrollback_len returns the number of scrollback rows
        let total_rows = visible_rows + screen.scrollback();
        total_rows.saturating_sub(visible_rows)
    }

    /// Mark the process as exited
    pub fn mark_exited(&mut self, exit_code: Option<i32>) {
        self.exit_code = exit_code;
        self.active = false;
    }

    /// Kill the child process
    pub fn kill(&mut self) {
        // Signal the reader thread to stop
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);

        // Kill the child process - this should cause the PTY to close
        let _ = self.child_killer.kill();
        self.active = false;

        // Don't block waiting for the reader thread - it will exit when:
        // 1. The PTY read returns EOF (from child exit)
        // 2. The PTY read returns an error
        // 3. The stop flag is checked on the next iteration
        // Blocking here could hang if the PTY doesn't close immediately.
        // The thread is detached and will clean up on its own.
        let _ = self.reader_handle.take();
    }

}

impl Drop for EmbeddedTerminal {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Convert vt100 color to ratatui color
pub fn vt100_to_ratatui_color(color: vt100::Color) -> ratatui::style::Color {
    match color {
        vt100::Color::Default => ratatui::style::Color::Reset,
        vt100::Color::Idx(idx) => {
            // Standard 16-color palette
            match idx {
                0 => ratatui::style::Color::Black,
                1 => ratatui::style::Color::Red,
                2 => ratatui::style::Color::Green,
                3 => ratatui::style::Color::Yellow,
                4 => ratatui::style::Color::Blue,
                5 => ratatui::style::Color::Magenta,
                6 => ratatui::style::Color::Cyan,
                7 => ratatui::style::Color::White,
                8 => ratatui::style::Color::DarkGray,
                9 => ratatui::style::Color::LightRed,
                10 => ratatui::style::Color::LightGreen,
                11 => ratatui::style::Color::LightYellow,
                12 => ratatui::style::Color::LightBlue,
                13 => ratatui::style::Color::LightMagenta,
                14 => ratatui::style::Color::LightCyan,
                15 => ratatui::style::Color::White,
                _ => ratatui::style::Color::Indexed(idx),
            }
        }
        vt100::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
    }
}

/// Convert vt100 cell to ratatui style
pub fn vt100_cell_to_style(cell: &vt100::Cell) -> ratatui::style::Style {
    let mut style = ratatui::style::Style::default();

    style = style.fg(vt100_to_ratatui_color(cell.fgcolor()));
    style = style.bg(vt100_to_ratatui_color(cell.bgcolor()));

    if cell.bold() {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(ratatui::style::Modifier::REVERSED);
    }

    style
}

/// Key sequence mapping for terminal input
pub mod key_sequences {
    /// Escape key
    pub const ESC: &[u8] = b"\x1b";
    /// Enter/Return
    pub const ENTER: &[u8] = b"\r";
    /// Backspace (DEL)
    pub const BACKSPACE: &[u8] = b"\x7f";
    /// Tab
    pub const TAB: &[u8] = b"\t";
    /// Ctrl+C (SIGINT)
    pub const CTRL_C: &[u8] = b"\x03";
    /// Ctrl+D (EOF)
    pub const CTRL_D: &[u8] = b"\x04";
    /// Ctrl+A (line start)
    pub const CTRL_A: &[u8] = b"\x01";
    /// Ctrl+E (line end)
    pub const CTRL_E: &[u8] = b"\x05";
    /// Ctrl+U (delete to start)
    pub const CTRL_U: &[u8] = b"\x15";
    /// Ctrl+K (delete to end)
    pub const CTRL_K: &[u8] = b"\x0b";
    /// Ctrl+L (clear screen)
    pub const CTRL_L: &[u8] = b"\x0c";
    /// Ctrl+W (delete word)
    pub const CTRL_W: &[u8] = b"\x17";
    /// Ctrl+Z (suspend)
    pub const CTRL_Z: &[u8] = b"\x1a";
    /// Arrow Up
    pub const ARROW_UP: &[u8] = b"\x1b[A";
    /// Arrow Down
    pub const ARROW_DOWN: &[u8] = b"\x1b[B";
    /// Arrow Right
    pub const ARROW_RIGHT: &[u8] = b"\x1b[C";
    /// Arrow Left
    pub const ARROW_LEFT: &[u8] = b"\x1b[D";
    /// Home
    pub const HOME: &[u8] = b"\x1b[H";
    /// End
    pub const END: &[u8] = b"\x1b[F";
    /// Delete
    pub const DELETE: &[u8] = b"\x1b[3~";
    /// Page Up
    pub const PAGE_UP: &[u8] = b"\x1b[5~";
    /// Page Down
    pub const PAGE_DOWN: &[u8] = b"\x1b[6~";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_conversion() {
        assert_eq!(
            vt100_to_ratatui_color(vt100::Color::Default),
            ratatui::style::Color::Reset
        );
        assert_eq!(
            vt100_to_ratatui_color(vt100::Color::Idx(1)),
            ratatui::style::Color::Red
        );
        assert_eq!(
            vt100_to_ratatui_color(vt100::Color::Rgb(255, 128, 64)),
            ratatui::style::Color::Rgb(255, 128, 64)
        );
    }

    #[test]
    fn test_key_sequences() {
        assert_eq!(key_sequences::ESC, b"\x1b");
        assert_eq!(key_sequences::CTRL_C, b"\x03");
        assert_eq!(key_sequences::ARROW_UP, b"\x1b[A");
    }
}
