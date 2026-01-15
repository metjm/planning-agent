//! Async daemon subscription for receiving push notifications.
//!
//! This module provides `DaemonSubscription` which uses tokio async I/O
//! to subscribe to session change events from the daemon.

#![allow(dead_code)]

use crate::planning_paths;
use crate::session_daemon::protocol::{ClientMessage, DaemonMessage};

/// Log a daemon-related event to ~/.planning-agent/daemon-debug.log
fn daemon_log(msg: &str) {
    use std::io::Write;
    if let Ok(home) = planning_paths::planning_agent_home_dir() {
        let log_path = home.join("daemon-debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "[{}] [subscription] {}", now, msg);
        }
    }
}

/// Async daemon subscription for receiving push notifications.
///
/// This uses tokio async I/O and provides a channel for receiving session updates.
pub struct DaemonSubscription {
    /// Receiver for session change events
    rx: tokio::sync::mpsc::UnboundedReceiver<DaemonMessage>,
    /// Handle to the background reader task
    _task: tokio::task::JoinHandle<()>,
    /// Keep writer alive to prevent socket close (Unix)
    #[cfg(unix)]
    _writer: tokio::net::unix::OwnedWriteHalf,
    /// Keep writer alive to prevent socket close (Windows)
    #[cfg(windows)]
    _writer: tokio::net::tcp::OwnedWriteHalf,
}

impl DaemonSubscription {
    /// Create a new subscription to the daemon.
    /// Returns None if unable to connect.
    #[cfg(unix)]
    pub async fn connect() -> Option<Self> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixStream;

        daemon_log("connect() called");

        let socket_path = planning_paths::sessiond_socket_path().ok()?;
        daemon_log(&format!("connecting to socket: {:?}", socket_path));

        let stream = match UnixStream::connect(&socket_path).await {
            Ok(s) => {
                daemon_log("socket connected successfully");
                s
            }
            Err(e) => {
                daemon_log(&format!("socket connect failed: {}", e));
                return None;
            }
        };

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send Subscribe message
        let subscribe_msg = serde_json::to_string(&ClientMessage::Subscribe).ok()?;
        daemon_log("sending Subscribe message");
        if let Err(e) = writer
            .write_all(format!("{}\n", subscribe_msg).as_bytes())
            .await
        {
            daemon_log(&format!("failed to send Subscribe: {}", e));
            return None;
        }

        // Wait for Subscribed confirmation
        daemon_log("waiting for Subscribed confirmation");
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                daemon_log("got EOF waiting for Subscribed");
                return None;
            }
            Ok(_) => {
                daemon_log(&format!("received: {}", line.trim()));
            }
            Err(e) => {
                daemon_log(&format!("read error waiting for Subscribed: {}", e));
                return None;
            }
        }

        let response: DaemonMessage = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                daemon_log(&format!("failed to parse response: {}", e));
                return None;
            }
        };

        if !matches!(response, DaemonMessage::Subscribed) {
            daemon_log(&format!("unexpected response: {:?}", response));
            return None;
        }
        daemon_log("subscription confirmed, starting reader task");

        // Create channel for forwarding events
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn background task to read events
        let task = tokio::spawn(async move {
            daemon_log("reader task started");
            let mut reader = reader;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        daemon_log("reader task: got EOF, exiting");
                        break;
                    }
                    Ok(_) => {
                        daemon_log(&format!("reader task: received: {}", line.trim()));
                        if let Ok(msg) = serde_json::from_str::<DaemonMessage>(line.trim()) {
                            daemon_log(&format!("reader task: parsed message: {:?}", msg));
                            if tx.send(msg).is_err() {
                                daemon_log("reader task: receiver dropped, exiting");
                                break;
                            }
                        } else {
                            daemon_log("reader task: failed to parse message");
                        }
                    }
                    Err(e) => {
                        daemon_log(&format!("reader task: read error: {}, exiting", e));
                        break;
                    }
                }
            }
            daemon_log("reader task ended");
        });

        daemon_log("connect() returning successfully");
        Some(Self {
            rx,
            _task: task,
            _writer: writer,
        })
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

        Some(Self {
            rx,
            _task: task,
            _writer: writer,
        })
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
