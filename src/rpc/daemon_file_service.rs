//! RPC service for host to request session files from daemon.

use serde::{Deserialize, Serialize};

/// File metadata for directory listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_at: String, // RFC3339 timestamp
}

/// Error type for file access operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FileAccessError {
    SessionNotFound,
    FileNotFound,
    PermissionDenied,
    IoError(String),
}

impl std::fmt::Display for FileAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => write!(f, "Session not found"),
            Self::FileNotFound => write!(f, "File not found"),
            Self::PermissionDenied => write!(f, "Permission denied"),
            Self::IoError(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for FileAccessError {}

/// Maximum file size to read (1MB) - larger files are truncated.
pub const MAX_FILE_READ_SIZE: usize = 1_048_576;

/// File content response, handling truncation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub content: String,
    pub truncated: bool,
    pub total_size: u64,
}

/// Service for host to access session files on daemon.
/// The daemon implements this service; the host calls it.
#[tarpc::service]
pub trait DaemonFileService {
    /// List files in a session directory.
    async fn list_session_files(session_id: String) -> Result<Vec<FileEntry>, FileAccessError>;

    /// Read the content of a file (returns up to MAX_FILE_READ_SIZE bytes as UTF-8).
    /// For binary files, returns error. For large files, truncates with indicator.
    async fn read_session_file(
        session_id: String,
        filename: String,
    ) -> Result<FileContent, FileAccessError>;
}
