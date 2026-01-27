//! Session detail panel for displaying comprehensive session information.

use crate::rpc::FileEntry;
use crate::tui::ui::util::format_bytes;

use super::session_table::LivenessDisplay;

/// Session detail data fetched via RPC.
#[derive(Default)]
pub struct SessionDetailData {
    pub session_id: String,
    pub feature_name: String,
    pub container_name: String,
    pub container_id: String,
    pub phase: String,
    pub iteration: u32,
    pub status: String,
    pub liveness: LivenessDisplay,
    pub pid: u32,
    pub updated_ago: String,
    pub files: Vec<FileEntryDisplay>,
    pub selected_file: Option<usize>,
    pub file_content: Option<FileContentDisplay>,
    pub loading_files: bool,
    pub loading_content: bool,
    pub error: Option<String>,
}

/// Display wrapper for FileEntry from RPC.
#[derive(Clone)]
pub struct FileEntryDisplay {
    pub name: String,
    pub is_dir: bool,
    pub size_display: String,
}

impl FileEntryDisplay {
    pub fn from_rpc(entry: &FileEntry) -> Self {
        Self {
            name: entry.name.clone(),
            is_dir: entry.is_dir,
            size_display: format_bytes(entry.size as usize),
        }
    }
}

/// Display wrapper for file content.
#[derive(Clone)]
pub struct FileContentDisplay {
    pub content: String,
    pub truncated: bool,
    pub total_size_display: String,
}

/// Render the session detail panel.
/// Returns (should_close, file_click) - file_click is (session_id, filename) if a file was clicked.
pub fn render_session_detail_panel(
    ui: &mut eframe::egui::Ui,
    detail: &mut SessionDetailData,
) -> (bool, Option<(String, String)>) {
    use eframe::egui;

    let mut should_close = false;
    let mut file_click: Option<(String, String)> = None;

    // Header with container name prominently displayed and close button
    ui.horizontal(|ui| {
        ui.heading("Session Details");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                should_close = true;
            }
        });
    });
    // Container name prominently displayed below header
    ui.horizontal(|ui| {
        ui.label("📦");
        ui.strong(&detail.container_name);
    });
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Status section
        ui.strong("Status");
        ui.horizontal(|ui| {
            let color = super::session_table::liveness_color(detail.liveness);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, color);
            ui.label(format!("{:?}", detail.liveness));
        });
        ui.label(format!("Phase: {}", detail.phase));
        ui.label(format!("Iteration: {}", detail.iteration));
        ui.label(format!("PID: {}", detail.pid));
        ui.label(format!("Updated: {}", detail.updated_ago));
        ui.add_space(8.0);

        // Info section
        ui.strong("Info");
        ui.label(format!("Feature: {}", detail.feature_name));
        ui.add_space(8.0);

        // Error display
        if let Some(error) = &detail.error {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::RED, "⚠");
                ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
            });
            ui.add_space(4.0);
        }

        // Files section
        ui.horizontal(|ui| {
            ui.strong("Session Files");
            if detail.loading_files {
                ui.spinner();
            }
        });
        ui.separator();

        for (idx, file) in detail.files.iter().enumerate() {
            ui.horizontal(|ui| {
                let icon = if file.is_dir { "📁" } else { "📄" };
                ui.label(icon);

                let is_selected = detail.selected_file == Some(idx);
                if ui.selectable_label(is_selected, &file.name).clicked() && !file.is_dir {
                    detail.selected_file = Some(idx);
                    detail.loading_content = true;
                    detail.file_content = None;
                    file_click = Some((detail.session_id.clone(), file.name.clone()));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(&file.size_display);
                });
            });
        }

        // File content preview
        if let Some(content) = &detail.file_content {
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("File Content");
                if detail.loading_content {
                    ui.spinner();
                }
                if ui
                    .small_button("📋 Copy")
                    .on_hover_text("Copy content")
                    .clicked()
                {
                    ui.ctx().copy_text(content.content.clone());
                }
            });

            if content.truncated {
                ui.small(format!("(truncated, {} total)", content.total_size_display));
            }

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .id_salt("file_content_scroll")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut content.content.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });
        }
    });

    (should_close, file_click)
}
