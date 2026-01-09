//! Implementation terminal input handling for the embedded Claude Code terminal.

use crate::tui::embedded_terminal::{key_sequences, EmbeddedTerminal, MIN_TERMINAL_COLS, MIN_TERMINAL_ROWS};
use crate::tui::{Event, FocusedPanel, InputMode, Session};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Start the embedded implementation terminal
pub fn start_implementation_terminal(
    session: &mut Session,
    plan_path: PathBuf,
    working_dir: &Path,
    output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<()> {
    // Check if Claude CLI is available
    if which::which("claude").is_err() {
        anyhow::bail!("Claude CLI not found. Please install it to use implementation mode.");
    }

    // Get terminal size
    let (term_width, term_height) = crossterm::terminal::size().unwrap_or((80, 24));

    // Validate minimum size
    if term_width < MIN_TERMINAL_COLS || term_height < MIN_TERMINAL_ROWS {
        anyhow::bail!(
            "Terminal too small for implementation mode. Minimum size: {}x{}, current: {}x{}",
            MIN_TERMINAL_COLS,
            MIN_TERMINAL_ROWS,
            term_width,
            term_height
        );
    }

    // Compute terminal panel size (approximation - similar to draw_main layout)
    // Main layout: top bar (2) + footer (3) = 5 rows overhead
    // Horizontal: 70% for left panel
    let panel_height = term_height.saturating_sub(5);
    let panel_width = (term_width as f32 * 0.70) as u16;

    // Account for borders (2 rows, 2 cols) to get inner terminal size
    let inner_height = panel_height.saturating_sub(2);
    let inner_width = panel_width.saturating_sub(2);

    // Spawn the embedded terminal with inner dimensions
    let terminal = EmbeddedTerminal::spawn(
        &plan_path,
        working_dir,
        inner_height,
        inner_width,
        session.id,
        output_tx.clone(),
    ).context("Failed to spawn embedded implementation terminal")?;

    session.implementation_terminal = Some(terminal);
    session.input_mode = InputMode::ImplementationTerminal;
    session.focused_panel = FocusedPanel::Implementation;

    session.add_output(format!(
        "[implementation] Starting Claude Code for: {}",
        plan_path.display()
    ));

    Ok(())
}

/// Handle keyboard input when in implementation terminal mode
pub fn handle_implementation_terminal_input(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) -> Result<bool> {
    let impl_term = match &session.implementation_terminal {
        Some(term) => term,
        None => {
            // No terminal, return to normal mode
            session.input_mode = InputMode::Normal;
            return Ok(false);
        }
    };

    // Check for exit sequence: Ctrl+\ (SIGQUIT)
    if key.code == KeyCode::Char('\\') && key.modifiers.contains(KeyModifiers::CONTROL) {
        session.stop_implementation_terminal();
        session.add_output("[implementation] Terminal closed by user".to_string());
        return Ok(false);
    }

    // Map key to bytes and send to PTY
    let bytes: Option<Vec<u8>> = match key.code {
        KeyCode::Char(c) => {
            // Handle Ctrl+key combinations
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    'c' | 'C' => Some(key_sequences::CTRL_C.to_vec()),
                    'd' | 'D' => Some(key_sequences::CTRL_D.to_vec()),
                    'a' | 'A' => Some(key_sequences::CTRL_A.to_vec()),
                    'e' | 'E' => Some(key_sequences::CTRL_E.to_vec()),
                    'u' | 'U' => Some(key_sequences::CTRL_U.to_vec()),
                    'k' | 'K' => Some(key_sequences::CTRL_K.to_vec()),
                    'l' | 'L' => Some(key_sequences::CTRL_L.to_vec()),
                    'w' | 'W' => Some(key_sequences::CTRL_W.to_vec()),
                    'z' | 'Z' => Some(key_sequences::CTRL_Z.to_vec()),
                    _ => None,
                }
            } else {
                // Regular character
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                Some(s.as_bytes().to_vec())
            }
        }
        KeyCode::Enter => Some(key_sequences::ENTER.to_vec()),
        KeyCode::Backspace => Some(key_sequences::BACKSPACE.to_vec()),
        KeyCode::Tab => Some(key_sequences::TAB.to_vec()),
        KeyCode::Esc => Some(key_sequences::ESC.to_vec()),
        KeyCode::Up => Some(key_sequences::ARROW_UP.to_vec()),
        KeyCode::Down => Some(key_sequences::ARROW_DOWN.to_vec()),
        KeyCode::Left => Some(key_sequences::ARROW_LEFT.to_vec()),
        KeyCode::Right => Some(key_sequences::ARROW_RIGHT.to_vec()),
        KeyCode::Home => Some(key_sequences::HOME.to_vec()),
        KeyCode::End => Some(key_sequences::END.to_vec()),
        KeyCode::Delete => Some(key_sequences::DELETE.to_vec()),
        KeyCode::PageUp => Some(key_sequences::PAGE_UP.to_vec()),
        KeyCode::PageDown => Some(key_sequences::PAGE_DOWN.to_vec()),
        _ => None,
    };

    if let Some(bytes) = bytes {
        if let Err(e) = impl_term.send_input(&bytes) {
            session.add_output(format!("[implementation] Input error: {}", e));
        }
    }

    Ok(false)
}

/// Handle scroll input for implementation terminal when not in terminal input mode
#[allow(dead_code)]
pub fn handle_implementation_terminal_scroll(
    key: crossterm::event::KeyEvent,
    session: &mut Session,
) {
    if let Some(ref mut impl_term) = session.implementation_terminal {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                impl_term.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                impl_term.scroll_up();
            }
            KeyCode::Char('g') => {
                impl_term.scroll_to_top();
            }
            KeyCode::Char('G') => {
                impl_term.scroll_to_bottom();
            }
            _ => {}
        }
    }
}
