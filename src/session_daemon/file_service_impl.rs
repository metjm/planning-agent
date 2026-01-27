//! Implementation of DaemonFileService for the daemon.

use crate::planning_paths;
use crate::rpc::daemon_file_service::{
    DaemonFileService, FileAccessError, FileContent, FileEntry, MAX_FILE_READ_SIZE,
};
use std::fs;

/// Implementation of DaemonFileService.
/// Clone is required because tarpc 0.37 takes ownership of self on each call.
#[derive(Clone)]
pub struct DaemonFileServer;

impl DaemonFileServer {
    pub fn new() -> Self {
        Self
    }
}

/// Get session directory path WITHOUT creating it.
/// Unlike planning_paths::session_dir(), this does not call create_dir_all().
fn session_dir_path(session_id: &str) -> Result<std::path::PathBuf, FileAccessError> {
    let home = planning_paths::planning_agent_home_dir()
        .map_err(|e| FileAccessError::IoError(e.to_string()))?;
    Ok(home.join("sessions").join(session_id))
}

impl DaemonFileService for DaemonFileServer {
    async fn list_session_files(
        self,
        _: tarpc::context::Context,
        session_id: String,
    ) -> Result<Vec<FileEntry>, FileAccessError> {
        let session_dir = session_dir_path(&session_id)?;

        if !session_dir.exists() {
            return Err(FileAccessError::SessionNotFound);
        }

        let mut entries = Vec::new();
        let read_dir =
            fs::read_dir(&session_dir).map_err(|e| FileAccessError::IoError(e.to_string()))?;

        for entry in read_dir.filter_map(|e| e.ok()) {
            let metadata = entry.metadata().ok();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified_at = metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
                .unwrap_or_default();

            entries.push(FileEntry {
                name,
                is_dir,
                size,
                modified_at,
            });
        }

        // Sort: directories first, then by name
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    async fn read_session_file(
        self,
        _: tarpc::context::Context,
        session_id: String,
        filename: String,
    ) -> Result<FileContent, FileAccessError> {
        // ========== SECURITY: Path Traversal Prevention ==========
        // Layer 1a: Reject null bytes
        if filename.contains('\0') {
            return Err(FileAccessError::PermissionDenied);
        }

        // Layer 1b: Reject path separators
        if filename
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '\u{2215}' | '\u{2044}'))
        {
            return Err(FileAccessError::PermissionDenied);
        }

        // Layer 1c: Reject if Path sees multiple components
        let path = std::path::Path::new(&filename);
        if path.components().count() != 1 || path.is_absolute() {
            return Err(FileAccessError::PermissionDenied);
        }

        let session_dir = session_dir_path(&session_id)?;

        if !session_dir.exists() {
            return Err(FileAccessError::SessionNotFound);
        }

        let file_path = session_dir.join(&filename);

        // Layer 2: AUTHORITATIVE SECURITY GATE - canonical path verification
        let canonical_path = file_path
            .canonicalize()
            .map_err(|_| FileAccessError::FileNotFound)?;
        let canonical_session_dir = session_dir
            .canonicalize()
            .map_err(|e| FileAccessError::IoError(e.to_string()))?;
        if !canonical_path.starts_with(&canonical_session_dir) {
            return Err(FileAccessError::PermissionDenied);
        }

        if !file_path.exists() {
            return Err(FileAccessError::FileNotFound);
        }

        let metadata =
            fs::metadata(&file_path).map_err(|e| FileAccessError::IoError(e.to_string()))?;
        let total_size = metadata.len();

        let content = fs::read_to_string(&file_path)
            .map_err(|e| FileAccessError::IoError(format!("Cannot read file: {}", e)))?;

        let (content, truncated) = if content.len() > MAX_FILE_READ_SIZE {
            let truncated_content: String = content.chars().take(MAX_FILE_READ_SIZE).collect();
            (truncated_content, true)
        } else {
            (content, false)
        };

        Ok(FileContent {
            content,
            truncated,
            total_size,
        })
    }
}
