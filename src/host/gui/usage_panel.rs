//! Usage panel rendering for host GUI.

use crate::account_usage::types::AccountId;
use eframe::egui;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AccountProvider {
    Claude,
    Codex,
    Gemini,
}

impl AccountProvider {
    pub fn try_from_str(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
        }
    }

    pub fn badge_color(&self) -> egui::Color32 {
        match self {
            Self::Claude => egui::Color32::from_rgb(216, 152, 96),
            Self::Gemini => egui::Color32::from_rgb(66, 133, 244),
            Self::Codex => egui::Color32::from_rgb(16, 163, 127),
        }
    }

    pub fn order_index(&self) -> u8 {
        match self {
            Self::Claude => 0,
            Self::Codex => 1,
            Self::Gemini => 2,
        }
    }
}

/// Display data for an account in the usage panel.
#[derive(Debug, Clone)]
pub struct DisplayAccountRow {
    pub account_id: AccountId,
    pub provider: AccountProvider,
    pub email: String,
    pub session_percent: Option<u8>,
    pub session_reset: String,
    pub weekly_percent: Option<u8>,
    pub weekly_reset: String,
    pub token_valid: bool,
}

/// Render the usage panel contents.
pub fn render_usage_panel_content(ui: &mut egui::Ui, accounts: &[DisplayAccountRow]) {
    if accounts.is_empty() {
        ui.label("No accounts tracked");
        ui.small("Usage data appears when");
        ui.small("credentials are detected");
        return;
    }

    let mut last_provider: Option<&AccountProvider> = None;
    for account in accounts {
        if last_provider.map_or(false, |provider| provider != &account.provider) {
            ui.separator();
        }

        ui.push_id(&account.account_id, |ui| {
            // Provider badge and email
            ui.horizontal(|ui| {
                ui.colored_label(account.provider.badge_color(), account.provider.label());
                if !account.token_valid {
                    ui.colored_label(egui::Color32::RED, "⚠");
                }
            });
            ui.small(&account.email);

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

            ui.add_space(8.0);
        });

        last_provider = Some(&account.provider);
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
