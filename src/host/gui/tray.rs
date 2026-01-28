//! System tray icon support.
//!
//! Provides a menu bar tray icon with session count and notifications.
//! Only available on macOS and Windows (gtk3-rs on Linux is deprecated).
//! Requires the `host-gui-tray` feature.

#![cfg(feature = "tray-icon")]

/// Icon dimensions: width accommodates two 7-pixel digits plus padding and separator
/// Layout: [3px pad][7px digit][8px gap][7px digit][3px pad] = 28px
const ICON_WIDTH: u32 = 28;
const ICON_HEIGHT: u32 = 22;

/// Digit dimensions for the 7x11 bitmaps
const DIGIT_WIDTH: u32 = 7;
const DIGIT_HEIGHT: u32 = 11;

/// 7x11 pixel digit bitmaps for 0-9 (each row is a byte, 7 LSBs represent pixels).
/// Format: 7 pixels wide, 11 pixels tall - approximately 2.3x larger than previous 3x5.
const DIGIT_BITMAPS: [[u8; 11]; 10] = [
    // 0: rounded rectangle with hollow center
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
    // 1: vertical line with base
    [
        0b0011000, //   ##
        0b0111000, //  ###
        0b1111000, // ####
        0b0011000, //   ##
        0b0011000, //   ##
        0b0011000, //   ##
        0b0011000, //   ##
        0b0011000, //   ##
        0b0011000, //   ##
        0b1111111, // #######
        0b1111111, // #######
    ],
    // 2: curved top, diagonal, flat bottom
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100011, // ##   ##
        0b0000011, //      ##
        0b0000110, //     ##
        0b0011100, //   ###
        0b0110000, //  ##
        0b1100000, // ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b1111111, // #######
    ],
    // 3: two curves stacked
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100011, // ##   ##
        0b0000011, //      ##
        0b0011110, //   ####
        0b0011110, //   ####
        0b0000011, //      ##
        0b0000011, //      ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
    // 4: L-shape with vertical
    [
        0b0000110, //     ##
        0b0001110, //    ###
        0b0011110, //   ####
        0b0110110, //  ## ##
        0b1100110, // ##  ##
        0b1111111, // #######
        0b1111111, // #######
        0b0000110, //     ##
        0b0000110, //     ##
        0b0000110, //     ##
        0b0000110, //     ##
    ],
    // 5: flat top, curve bottom
    [
        0b1111111, // #######
        0b1111111, // #######
        0b1100000, // ##
        0b1100000, // ##
        0b1111110, // ######
        0b1111111, // #######
        0b0000011, //      ##
        0b0000011, //      ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
    // 6: curve with enclosed bottom
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100000, // ##
        0b1100000, // ##
        0b1111110, // ######
        0b1111111, // #######
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
    // 7: flat top, diagonal down
    [
        0b1111111, // #######
        0b1111111, // #######
        0b0000011, //      ##
        0b0000110, //     ##
        0b0001100, //    ##
        0b0011000, //   ##
        0b0011000, //   ##
        0b0110000, //  ##
        0b0110000, //  ##
        0b0110000, //  ##
        0b0110000, //  ##
    ],
    // 8: two stacked circles
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b0111110, //  #####
        0b0111110, //  #####
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
    // 9: enclosed top, curve bottom
    [
        0b0111110, //  #####
        0b1111111, // #######
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1100011, // ##   ##
        0b1111111, // #######
        0b0111111, //  ######
        0b0000011, //      ##
        0b0000011, //      ##
        0b1111111, // #######
        0b0111110, //  #####
    ],
];

/// Draw a single digit at the given position in the RGBA buffer.
fn draw_digit(
    rgba: &mut [u8],
    width: u32,
    digit: usize,
    x_offset: u32,
    y_offset: u32,
    color: [u8; 4],
) {
    let bitmap = &DIGIT_BITMAPS[digit.min(9)];
    for (row_idx, &row_bits) in bitmap.iter().enumerate() {
        for col in 0..DIGIT_WIDTH {
            // Check bit from MSB side (bit 6 is leftmost pixel)
            if (row_bits >> (DIGIT_WIDTH - 1 - col)) & 1 == 1 {
                let x = x_offset + col;
                let y = y_offset + row_idx as u32;
                if x < width && y < ICON_HEIGHT {
                    let idx = ((y * width + x) * 4) as usize;
                    if idx + 3 < rgba.len() {
                        rgba[idx..idx + 4].copy_from_slice(&color);
                    }
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

/// Check if a point is inside a rounded rectangle.
fn is_in_rounded_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> bool {
    // Check corners
    let in_left = x < radius;
    let in_right = x >= width - radius;
    let in_top = y < radius;
    let in_bottom = y >= height - radius;

    // If in a corner region, check distance from corner center
    if in_left && in_top {
        let dx = x - radius;
        let dy = y - radius;
        return dx * dx + dy * dy <= radius * radius;
    }
    if in_right && in_top {
        let dx = x - (width - radius);
        let dy = y - radius;
        return dx * dx + dy * dy <= radius * radius;
    }
    if in_left && in_bottom {
        let dx = x - radius;
        let dy = y - (height - radius);
        return dx * dx + dy * dy <= radius * radius;
    }
    if in_right && in_bottom {
        let dx = x - (width - radius);
        let dy = y - (height - radius);
        return dx * dx + dy * dy <= radius * radius;
    }

    // Not in a corner, so it's in the rectangle
    true
}

/// Create a status icon showing running/awaiting counts as numbers.
/// Layout: Green number (running) on left, Amber number (awaiting) on right.
/// Numbers clamped to single digit (9+ shown as "9").
fn create_status_icon(running: usize, awaiting: usize) -> anyhow::Result<tray_icon::Icon> {
    let mut rgba = vec![0u8; (ICON_WIDTH * ICON_HEIGHT * 4) as usize];

    // Colors
    let green = [76u8, 175, 80, 255];
    let amber = [255u8, 152, 0, 255];
    let gray = [117u8, 117, 117, 255];
    let bg_dark = [40u8, 40, 40, 255];

    // Draw rounded rectangle background
    let corner_radius = 4.0f32;
    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            let idx = ((y * ICON_WIDTH + x) * 4) as usize;

            // Check if point is inside rounded rectangle
            let in_rect = is_in_rounded_rect(
                x as f32,
                y as f32,
                ICON_WIDTH as f32,
                ICON_HEIGHT as f32,
                corner_radius,
            );

            if in_rect {
                rgba[idx..idx + 4].copy_from_slice(&bg_dark);
            }
        }
    }

    // Clamp to single digits (show "9" for 9+)
    let running_digit = running.min(9);
    let awaiting_digit = awaiting.min(9);

    // Vertical centering: (22 - 11) / 2 = 5.5, round to 5
    let y_offset = (ICON_HEIGHT - DIGIT_HEIGHT) / 2;

    // Horizontal layout:
    // Left digit at x=3 (3px padding from left edge)
    // Right digit at x=18 (3px padding from right edge: 28-7-3=18)
    let left_x = 3;
    let right_x = ICON_WIDTH - DIGIT_WIDTH - 3;

    // Draw running count (green or gray if zero) - left side
    let running_color = if running > 0 { green } else { gray };
    draw_digit(
        &mut rgba,
        ICON_WIDTH,
        running_digit,
        left_x,
        y_offset,
        running_color,
    );

    // Draw awaiting count (amber or gray if zero) - right side
    let awaiting_color = if awaiting > 0 { amber } else { gray };
    draw_digit(
        &mut rgba,
        ICON_WIDTH,
        awaiting_digit,
        right_x,
        y_offset,
        awaiting_color,
    );

    Ok(tray_icon::Icon::from_rgba(rgba, ICON_WIDTH, ICON_HEIGHT)?)
}
