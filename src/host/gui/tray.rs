//! System tray icon support.
//!
//! Provides a menu bar tray icon with session count and notifications.

use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use std::sync::mpsc;
use tray_icon::{TrayIcon, TrayIconBuilder};

/// Commands from the tray menu.
#[derive(Debug, Clone)]
pub enum TrayCommand {
    ShowWindow,
    Quit,
}

/// Tray icon state and handler.
pub struct HostTray {
    _tray_icon: TrayIcon,
    command_rx: mpsc::Receiver<TrayCommand>,
}

impl HostTray {
    /// Create a new tray icon.
    /// Must be called on the main thread (macOS requirement).
    pub fn new() -> anyhow::Result<Self> {
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

        // Create tray icon with a simple programmatic icon
        let icon = create_simple_icon()?;

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Planning Agent Host")
            .with_icon(icon)
            .build()?;

        // Handle menu events
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                let _ = command_tx.send(TrayCommand::ShowWindow);
            } else if event.id == quit_id {
                let _ = command_tx.send(TrayCommand::Quit);
            }
        }));

        Ok(Self {
            _tray_icon: tray_icon,
            command_rx,
        })
    }

    /// Update the tray tooltip with session count.
    /// Note: tray-icon doesn't support updating tooltip after creation in v1.
    pub fn update_tooltip(&self, active_sessions: usize, approval_count: usize) {
        // This is a no-op in v1 of tray-icon - tooltip cannot be updated after creation
        let _ = (active_sessions, approval_count);
    }

    /// Try to receive a tray command (non-blocking).
    pub fn try_recv_command(&self) -> Option<TrayCommand> {
        self.command_rx.try_recv().ok()
    }
}

/// Create a simple 16x16 RGBA icon programmatically.
fn create_simple_icon() -> anyhow::Result<tray_icon::Icon> {
    let size = 16u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            // Simple circular gradient
            let cx = x as f32 - size as f32 / 2.0;
            let cy = y as f32 - size as f32 / 2.0;
            let dist = (cx * cx + cy * cy).sqrt();
            let radius = size as f32 / 2.0 - 1.0;

            if dist < radius {
                // Blue-ish color inside (matches planning-agent theme)
                rgba[idx] = 90; // R
                rgba[idx + 1] = 122; // G
                rgba[idx + 2] = 184; // B
                rgba[idx + 3] = 255; // A
            } else {
                // Transparent outside
                rgba[idx] = 0;
                rgba[idx + 1] = 0;
                rgba[idx + 2] = 0;
                rgba[idx + 3] = 0;
            }
        }
    }

    Ok(tray_icon::Icon::from_rgba(rgba, size, size)?)
}
