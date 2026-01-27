//! Session selection and detail panel management.
//!
//! Extracted from app.rs to keep files under the line limit.

use super::file_client;
use super::session_detail::{
    render_session_detail_panel, FileContentDisplay, FileEntryDisplay, SessionDetailData,
};
use super::session_table::DisplaySessionRow;
use crate::rpc::daemon_file_service::{FileContent, FileEntry};
use crate::tui::ui::util::format_bytes;
use eframe::egui;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Type alias for pending file list result (reduces type complexity).
pub type PendingFileList = Arc<Mutex<Option<Result<Vec<FileEntry>, String>>>>;

/// Type alias for pending file content result (reduces type complexity).
pub type PendingFileContent = Arc<Mutex<Option<Result<FileContent, String>>>>;

/// Display row for a container (needed for container lookup).
#[derive(Clone)]
pub struct DisplayContainerRowLite {
    pub container_id: String,
    pub container_name: String,
    pub file_service_port: u16,
}

/// Helper functions for session selection and detail management.
pub struct SessionSelectionManager;

impl SessionSelectionManager {
    /// Find container_id for a container by name.
    pub fn find_container_id_for_session(
        containers: &[DisplayContainerRowLite],
        container_name: &str,
    ) -> Option<String> {
        containers
            .iter()
            .find(|c| c.container_name == container_name)
            .map(|c| c.container_id.clone())
    }

    /// Get file service connection info for a container.
    pub fn get_file_service_info(
        containers: &[DisplayContainerRowLite],
        container_id: &str,
    ) -> Option<(String, u16)> {
        containers
            .iter()
            .find(|c| c.container_id == container_id)
            .map(|c| ("127.0.0.1".to_string(), c.file_service_port))
    }

    /// Spawn async task to fetch session files via RPC.
    pub fn fetch_session_files(
        pending: PendingFileList,
        host_port: Option<(String, u16)>,
        session_id: String,
    ) {
        let Some((host, port)) = host_port else {
            if let Ok(mut guard) = pending.try_lock() {
                *guard = Some(Err("Container not found".to_string()));
            }
            return;
        };

        if port == 0 {
            if let Ok(mut guard) = pending.try_lock() {
                *guard = Some(Err(
                    "File service not available (daemon too old?)".to_string()
                ));
            }
            return;
        }

        tokio::spawn(async move {
            let result = file_client::fetch_files_rpc(&host, port, &session_id).await;
            let mut guard = pending.lock().await;
            *guard = Some(result);
        });
    }

    /// Spawn async task to fetch file content via RPC.
    pub fn fetch_file_content(
        pending: PendingFileContent,
        host_port: Option<(String, u16)>,
        session_id: String,
        filename: String,
    ) {
        let Some((host, port)) = host_port else {
            if let Ok(mut guard) = pending.try_lock() {
                *guard = Some(Err("Container not found".to_string()));
            }
            return;
        };

        if port == 0 {
            if let Ok(mut guard) = pending.try_lock() {
                *guard = Some(Err("File service not available".to_string()));
            }
            return;
        }

        tokio::spawn(async move {
            let result = file_client::fetch_content_rpc(&host, port, &session_id, &filename).await;
            let mut guard = pending.lock().await;
            *guard = Some(result);
        });
    }

    /// Check for pending async results and update session_detail.
    pub fn check_pending_results(
        pending_file_list: &PendingFileList,
        pending_file_content: &PendingFileContent,
        session_detail: &mut Option<SessionDetailData>,
    ) {
        // Check file list result
        if let Ok(mut guard) = pending_file_list.try_lock() {
            if let Some(result) = guard.take() {
                if let Some(detail) = session_detail {
                    detail.loading_files = false;
                    match result {
                        Ok(files) => {
                            detail.files = files.iter().map(FileEntryDisplay::from_rpc).collect();
                            detail.error = None;
                        }
                        Err(e) => {
                            detail.error = Some(e);
                        }
                    }
                }
            }
        }

        // Check file content result
        if let Ok(mut guard) = pending_file_content.try_lock() {
            if let Some(result) = guard.take() {
                if let Some(detail) = session_detail {
                    detail.loading_content = false;
                    match result {
                        Ok(content) => {
                            detail.file_content = Some(FileContentDisplay {
                                content: content.content,
                                truncated: content.truncated,
                                total_size_display: format_bytes(content.total_size as usize),
                            });
                            detail.error = None;
                        }
                        Err(e) => {
                            detail.error = Some(e);
                        }
                    }
                }
            }
        }
    }

    /// Select a session and initiate RPC fetch for file list.
    /// Returns (new_selected_session_id, new_session_detail) tuple.
    pub fn select_session(
        session_id: &str,
        current_selected: &Option<String>,
        sessions: &[DisplaySessionRow],
        containers: &[DisplayContainerRowLite],
        pending_file_list: PendingFileList,
    ) -> (Option<String>, Option<SessionDetailData>) {
        // Toggle off if clicking same session
        if current_selected.as_ref() == Some(&session_id.to_string()) {
            return (None, None);
        }

        // Find session in display data
        let session = sessions.iter().find(|s| s.session_id == session_id);

        if let Some(session) = session {
            let container_id =
                Self::find_container_id_for_session(containers, &session.container_name)
                    .unwrap_or_default();

            let detail = SessionDetailData {
                session_id: session_id.to_string(),
                feature_name: session.feature_name.clone(),
                container_name: session.container_name.clone(),
                container_id: container_id.clone(),
                phase: session.phase.clone(),
                iteration: session.iteration,
                status: session.status.clone(),
                liveness: session.liveness,
                pid: session.pid,
                updated_ago: session.updated_ago.clone(),
                files: Vec::new(),
                selected_file: None,
                file_content: None,
                loading_files: true,
                loading_content: false,
                error: None,
            };

            // Start async file list fetch
            let host_port = Self::get_file_service_info(containers, &container_id);
            Self::fetch_session_files(pending_file_list, host_port, session_id.to_string());

            (Some(session_id.to_string()), Some(detail))
        } else {
            (None, None)
        }
    }

    /// Handle container disconnect while detail panel is open.
    pub fn handle_container_disconnect_for_detail(
        session_detail: &mut Option<SessionDetailData>,
        containers: &[DisplayContainerRowLite],
    ) {
        if let Some(detail) = session_detail {
            let container_exists = containers
                .iter()
                .any(|c| c.container_id == detail.container_id);

            if !container_exists {
                detail.error = Some("Container disconnected".to_string());
                detail.loading_files = false;
                detail.loading_content = false;
            }
        }
    }

    /// Sync detail panel data from display_data (keeps updated_ago fresh).
    pub fn sync_detail_from_display_data(
        session_detail: &mut Option<SessionDetailData>,
        sessions: &[DisplaySessionRow],
    ) {
        let Some(detail) = session_detail else {
            return;
        };

        if let Some(session) = sessions.iter().find(|s| s.session_id == detail.session_id) {
            detail.updated_ago = session.updated_ago.clone();
            detail.phase = session.phase.clone();
            detail.iteration = session.iteration;
            detail.status = session.status.clone();
            detail.liveness = session.liveness;
        }
    }

    /// Wrapper that delegates to session_detail::render_session_detail_panel
    /// and returns state (should_close, file_click).
    pub fn render_and_handle_detail_panel(
        ui: &mut egui::Ui,
        session_detail: &mut Option<SessionDetailData>,
        selected_session_id: &mut Option<String>,
        pending_file_content: PendingFileContent,
        containers: &[DisplayContainerRowLite],
    ) {
        let Some(mut detail) = session_detail.take() else {
            return;
        };

        let (should_close, file_click) = render_session_detail_panel(ui, &mut detail);

        if should_close {
            *selected_session_id = None;
            *session_detail = None;
            return;
        }

        let container_id = detail.container_id.clone();
        *session_detail = Some(detail);

        if let Some((session_id, filename)) = file_click {
            let host_port = Self::get_file_service_info(containers, &container_id);
            Self::fetch_file_content(pending_file_content, host_port, session_id, filename);
        }
    }
}
