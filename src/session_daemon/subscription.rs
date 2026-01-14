//! Async daemon subscription for receiving push notifications.
//!
//! This module provides `DaemonSubscription` which uses tokio async I/O
//! to subscribe to session change events from the daemon.

#![allow(dead_code)]

use crate::planning_paths;
use crate::session_daemon::protocol::{ClientMessage, DaemonMessage};

/// Async daemon subscription for receiving push notifications.
///
/// This uses tokio async I/O and provides a channel for receiving session updates.
pub struct DaemonSubscription {
    /// Receiver for session change events
    rx: tokio::sync::mpsc::UnboundedReceiver<DaemonMessage>,
    /// Handle to the background reader task
    _task: tokio::task::JoinHandle<()>,
}

impl DaemonSubscription {
    /// Create a new subscription to the daemon.
    /// Returns None if unable to connect.
    #[cfg(unix)]
    pub async fn connect() -> Option<Self> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixStream;

        let socket_path = planning_paths::sessiond_socket_path().ok()?;
        let stream = UnixStream::connect(&socket_path).await.ok()?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send Subscribe message
        let subscribe_msg = serde_json::to_string(&ClientMessage::Subscribe).ok()?;
        writer
            .write_all(format!("{}\n", subscribe_msg).as_bytes())
            .await
            .ok()?;

        // Wait for Subscribed confirmation
        let mut line = String::new();
        reader.read_line(&mut line).await.ok()?;
        let response: DaemonMessage = serde_json::from_str(line.trim()).ok()?;
        if !matches!(response, DaemonMessage::Subscribed) {
            return None;
        }

        // Create channel for forwarding events
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn background task to read events
        let task = tokio::spawn(async move {
            let mut reader = reader;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<DaemonMessage>(line.trim()) {
                            if tx.send(msg).is_err() {
                                break; // Receiver dropped
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Some(Self { rx, _task: task })
    }

    #[cfg(windows)]
    pub async fn connect() -> Option<Self> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpStream;

        // Read port file
        let port_path = planning_paths::sessiond_port_path().ok()?;
        let content = std::fs::read_to_string(&port_path).ok()?;
        let port_info: crate::session_daemon::protocol::PortFileContent =
            serde_json::from_str(&content).ok()?;

        let stream = TcpStream::connect(format!("127.0.0.1:{}", port_info.port))
            .await
            .ok()?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send auth token first
        writer
            .write_all(format!("{}\n", port_info.token).as_bytes())
            .await
            .ok()?;

        // Send Subscribe message
        let subscribe_msg = serde_json::to_string(&ClientMessage::Subscribe).ok()?;
        writer
            .write_all(format!("{}\n", subscribe_msg).as_bytes())
            .await
            .ok()?;

        // Wait for Subscribed confirmation
        let mut line = String::new();
        reader.read_line(&mut line).await.ok()?;
        let response: DaemonMessage = serde_json::from_str(line.trim()).ok()?;
        if !matches!(response, DaemonMessage::Subscribed) {
            return None;
        }

        // Create channel for forwarding events
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn background task to read events
        let task = tokio::spawn(async move {
            let mut reader = reader;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Ok(msg) = serde_json::from_str::<DaemonMessage>(line.trim()) {
                            if tx.send(msg).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Some(Self { rx, _task: task })
    }

    /// Receive the next event from the daemon.
    /// Returns None if the connection is closed.
    pub async fn recv(&mut self) -> Option<DaemonMessage> {
        self.rx.recv().await
    }

    /// Try to receive an event without blocking.
    pub fn try_recv(&mut self) -> Option<DaemonMessage> {
        self.rx.try_recv().ok()
    }
}
