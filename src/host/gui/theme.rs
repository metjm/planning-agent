//! Theme definitions for the host GUI.

use eframe::egui;

/// Status colors for sessions.
pub struct StatusColors;

impl StatusColors {
    pub fn running() -> egui::Color32 {
        egui::Color32::from_rgb(76, 175, 80) // Green
    }

    pub fn awaiting_approval() -> egui::Color32 {
        egui::Color32::from_rgb(255, 152, 0) // Orange
    }

    pub fn complete() -> egui::Color32 {
        egui::Color32::from_rgb(33, 150, 243) // Blue
    }

    pub fn error() -> egui::Color32 {
        egui::Color32::from_rgb(244, 67, 54) // Red
    }

    pub fn stopped() -> egui::Color32 {
        egui::Color32::from_rgb(117, 117, 117) // Gray
    }

    pub fn unknown() -> egui::Color32 {
        egui::Color32::from_rgb(158, 158, 158) // Light gray
    }
}

/// Configure the application's visual style.
pub fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    // Use dark theme
    style.visuals = egui::Visuals::dark();

    // Slightly larger default spacing
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);

    ctx.set_style(style);
}
