//! GUI components for host mode.
//!
//! Uses egui/eframe for the native desktop application.

pub mod app;
mod file_client;
mod helpers;
mod notifications;
pub mod session_detail;
mod session_selection;
mod session_table;
mod status_colors;
mod usage_extrapolation;
mod usage_panel;

#[cfg(feature = "tray-icon")]
pub mod tray;
