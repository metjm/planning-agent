//! Async RPC client for fetching session files from daemon.
//!
//! Extracted from app.rs to keep files under the line limit.

use crate::rpc::daemon_file_service::{DaemonFileServiceClient, FileContent, FileEntry};

/// Connect to a daemon's file service.
pub async fn connect_to_file_service(
    host: &str,
    port: u16,
) -> Result<DaemonFileServiceClient, String> {
    use tarpc::client;
    use tarpc::serde_transport::tcp;
    use tarpc::tokio_serde::formats::Bincode;

    let addr = format!("{}:{}", host, port);

    let transport = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tcp::connect(&addr, Bincode::default),
    )
    .await
    .map_err(|_| format!("Connection timeout to {}", addr))?
    .map_err(|e| format!("Connection failed to {}: {}", addr, e))?;

    Ok(DaemonFileServiceClient::new(client::Config::default(), transport).spawn())
}

/// Perform the actual RPC call to list files.
pub async fn fetch_files_rpc(
    host: &str,
    port: u16,
    session_id: &str,
) -> Result<Vec<FileEntry>, String> {
    let client = connect_to_file_service(host, port).await?;

    let result = client
        .list_session_files(tarpc::context::current(), session_id.to_string())
        .await
        .map_err(|e| format!("RPC error: {}", e))?;

    result.map_err(|e| format!("File access error: {:?}", e))
}

/// Perform the actual RPC call to read file content.
pub async fn fetch_content_rpc(
    host: &str,
    port: u16,
    session_id: &str,
    filename: &str,
) -> Result<FileContent, String> {
    let client = connect_to_file_service(host, port).await?;

    let result = client
        .read_session_file(
            tarpc::context::current(),
            session_id.to_string(),
            filename.to_string(),
        )
        .await
        .map_err(|e| format!("RPC error: {}", e))?;

    result.map_err(|e| format!("File read error: {:?}", e))
}
