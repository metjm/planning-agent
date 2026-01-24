//! Usage panel rendering for host GUI.

use super::helpers::truncate_path;
use eframe::egui;

/// Display data for an account in the usage panel.
#[derive(Debug, Clone)]
pub struct DisplayAccountRow {
    pub provider: String,
    pub email: String,
    pub session_percent: Option<u8>,
    pub session_reset: String,
    pub weekly_percent: Option<u8>,
    pub weekly_reset: String,
    pub token_valid: bool,
    pub error: Option<String>,
}

/// Render the usage panel contents.
pub fn render_usage_panel_content(ui: &mut egui::Ui, accounts: &[DisplayAccountRow]) {
    if accounts.is_empty() {
        ui.label("No accounts tracked");
        ui.small("Usage data appears when");
        ui.small("credentials are detected");
        return;
    }

    for account in accounts {
        ui.push_id(&account.email, |ui| {
            // Provider badge and email
            ui.horizontal(|ui| {
                let badge_color = match account.provider.as_str() {
                    "claude" => egui::Color32::from_rgb(216, 152, 96),
                    "gemini" => egui::Color32::from_rgb(66, 133, 244),
                    "codex" => egui::Color32::from_rgb(16, 163, 127),
                    _ => egui::Color32::GRAY,
                };
                ui.colored_label(badge_color, &account.provider);
                if !account.token_valid {
                    ui.colored_label(egui::Color32::RED, "⚠");
                }
            });
            ui.small(&account.email);

            // Error display
            if let Some(err) = &account.error {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), "Error:");
                    if ui
                        .small_button("📋")
                        .on_hover_text("Copy full error")
                        .clicked()
                    {
                        ui.ctx().copy_text(err.clone());
                    }
                });
                ui.small(truncate_path(err, 30));
            } else {
                // Session usage bar
                if let Some(pct) = account.session_percent {
                    ui.horizontal(|ui| {
                        ui.small("Session:");
                        render_usage_bar(ui, pct);
                        if !account.session_reset.is_empty() {
                            ui.small(&account.session_reset);
                        }
                    });
                }
                // Weekly usage bar
                if let Some(pct) = account.weekly_percent {
                    ui.horizontal(|ui| {
                        ui.small("Weekly:");
                        render_usage_bar(ui, pct);
                        if !account.weekly_reset.is_empty() {
                            ui.small(&account.weekly_reset);
                        }
                    });
                }
            }

            ui.add_space(8.0);
        });
    }
}

/// Render a small usage progress bar.
pub fn render_usage_bar(ui: &mut egui::Ui, percent: u8) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(50.0, 8.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 2.0, egui::Color32::from_rgb(60, 60, 60));
    let fill_color = match percent {
        90..=100 => egui::Color32::from_rgb(244, 67, 54),
        70..=89 => egui::Color32::from_rgb(255, 152, 0),
        _ => egui::Color32::from_rgb(76, 175, 80),
    };
    let fill_rect =
        egui::Rect::from_min_size(rect.min, egui::vec2(50.0 * percent as f32 / 100.0, 8.0));
    ui.painter().rect_filled(fill_rect, 2.0, fill_color);
    ui.small(format!("{}%", percent));
}
