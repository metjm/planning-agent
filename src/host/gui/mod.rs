//! GUI components for host mode.
//!
//! Uses egui/eframe for the native desktop application.

pub mod app;
mod helpers;

#[cfg(feature = "tray-icon")]
pub mod tray;
