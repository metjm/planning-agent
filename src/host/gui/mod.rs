//! GUI components for host mode.
//!
//! Uses egui/eframe for the native desktop application.

pub mod app;
mod helpers;
mod status_colors;
mod usage_panel;

#[cfg(feature = "tray-icon")]
pub mod tray;
