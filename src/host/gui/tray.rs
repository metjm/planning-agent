//! System tray icon support.
//!
//! Provides a menu bar tray icon with session count and notifications.
//! Only available on macOS and Windows (gtk3-rs on Linux is deprecated).
//! Requires the `host-gui-tray` feature.

#![cfg(feature = "tray-icon")]

/// Simple 3x5 digit bitmaps for 0-9 (each row is a byte, bits represent pixels).
/// Format: 3 pixels wide, 5 pixels tall.
const DIGIT_BITMAPS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

/// Draw a single digit at the given position in the RGBA buffer.
fn draw_digit(
    rgba: &mut [u8],
    size: u32,
    digit: usize,
    x_offset: u32,
    y_offset: u32,
    color: [u8; 4],
) {
    let bitmap = &DIGIT_BITMAPS[digit.min(9)];
    for (row_idx, &row_bits) in bitmap.iter().enumerate() {
        for col in 0..3u32 {
            if (row_bits >> (2 - col)) & 1 == 1 {
                let x = x_offset + col;
                let y = y_offset + row_idx as u32;
                if x < size && y < size {
                    let idx = ((y * size + x) * 4) as usize;
                    rgba[idx..idx + 4].copy_from_slice(&color);
                }
            }
        }
    }
}

/// Commands from the tray menu.
#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowWindow,
    Quit,
}

/// Tray icon state and handler.
pub struct HostTray {
    tray_icon: tray_icon::TrayIcon,
    command_rx: std::sync::mpsc::Receiver<TrayCommand>,
    /// Cached counts to avoid unnecessary icon updates
    last_running: usize,
    last_awaiting: usize,
}

impl HostTray {
    /// Create a new tray icon.
    /// Must be called on the main thread (macOS requirement).
    pub fn new() -> anyhow::Result<Self> {
        use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use std::sync::mpsc;
        use tray_icon::TrayIconBuilder;

        let (command_tx, command_rx) = mpsc::channel();

        // Create menu
        let menu = Menu::new();

        let show_item = MenuItem::new("Show Dashboard", true, None);
        let show_id = show_item.id().clone();

        let quit_item = MenuItem::new("Quit", true, None);
        let quit_id = quit_item.id().clone();

        menu.append(&show_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        // Create tray icon with status icon showing counts
        let icon = create_status_icon(0, 0)?;

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Planning Agent Host - 0 running, 0 awaiting")
            .with_icon(icon)
            .build()?;

        // Handle menu events
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                // Ignoring send error: receiver may have been dropped if app is shutting down
                let _ = command_tx.send(TrayCommand::ShowWindow);
            } else if event.id == quit_id {
                // Ignoring send error: receiver may have been dropped if app is shutting down
                let _ = command_tx.send(TrayCommand::Quit);
            }
        }));

        Ok(Self {
            tray_icon,
            command_rx,
            last_running: 0,
            last_awaiting: 0,
        })
    }

    /// Try to receive a tray command (non-blocking).
    pub fn try_recv_command(&self) -> Option<TrayCommand> {
        self.command_rx.try_recv().ok()
    }

    /// Update the tray icon and tooltip to reflect current session counts.
    /// Only updates if counts have changed to avoid unnecessary redraws.
    pub fn update_icon(&mut self, running: usize, awaiting: usize) {
        // Skip update if counts haven't changed
        if running == self.last_running && awaiting == self.last_awaiting {
            return;
        }

        // Update cached counts
        self.last_running = running;
        self.last_awaiting = awaiting;

        // Update icon
        if let Ok(icon) = create_status_icon(running, awaiting) {
            // Ignoring error: icon update failures are non-critical
            let _ = self.tray_icon.set_icon(Some(icon));
        }

        // Update tooltip with exact counts (for when numbers exceed single digit)
        let tooltip = format!(
            "Planning Agent Host - {} running, {} awaiting",
            running, awaiting
        );
        // Ignoring error: tooltip update failures are non-critical (unsupported on Linux)
        let _ = self.tray_icon.set_tooltip(Some(&tooltip));
    }
}

/// Create a 22x22 icon showing running/awaiting counts as numbers.
/// Layout: Green number (running) on left, Red/Amber number (awaiting) on right.
/// Numbers clamped to single digit (9+ shown as "9").
fn create_status_icon(running: usize, awaiting: usize) -> anyhow::Result<tray_icon::Icon> {
    let size = 22u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    // Colors
    let green = [76u8, 175, 80, 255];
    let amber = [255u8, 152, 0, 255];
    let gray = [117u8, 117, 117, 255];

    // Draw circular background
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let cx = x as f32 - size as f32 / 2.0;
            let cy = y as f32 - size as f32 / 2.0;
            let dist = (cx * cx + cy * cy).sqrt();
            let radius = size as f32 / 2.0 - 1.0;

            if dist < radius {
                // Dark background inside circle
                rgba[idx] = 40;
                rgba[idx + 1] = 40;
                rgba[idx + 2] = 40;
                rgba[idx + 3] = 255;
            }
        }
    }

    // Clamp to single digits (show "9" for 9+)
    let running_digit = running.min(9);
    let awaiting_digit = awaiting.min(9);

    // Draw running count (green) - left side, vertically centered
    let running_color = if running > 0 { green } else { gray };
    draw_digit(&mut rgba, size, running_digit, 4, 8, running_color);

    // Draw awaiting count (amber/red) - right side, vertically centered
    let awaiting_color = if awaiting > 0 { amber } else { gray };
    draw_digit(&mut rgba, size, awaiting_digit, 15, 8, awaiting_color);

    Ok(tray_icon::Icon::from_rgba(rgba, size, size)?)
}
