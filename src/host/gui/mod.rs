//! GUI components for host mode.
//!
//! Uses egui/eframe for the native desktop application.

pub mod app;

#[cfg(not(target_os = "linux"))]
pub mod tray;
