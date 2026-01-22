//! Upstream connection to host application.
//!
//! This module manages the connection from a container daemon to the host
//! application for session aggregation. It:
//! - Connects to the host on port 17717 (or PLANNING_AGENT_HOST_PORT)
//! - Sends session updates as they occur
//! - Handles disconnection and reconnection with exponential backoff
//! - Sends periodic heartbeats
//!
//! This module is integrated with the daemon server and enabled when
//! PLANNING_AGENT_HOST_PORT is set.

use crate::host_protocol::{DaemonToHost, HostToDaemon, SessionInfo, PROTOCOL_VERSION};
use crate::session_daemon::SessionRecord;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// Events to send upstream to host.
#[derive(Debug, Clone)]
pub enum UpstreamEvent {
    /// Sync all sessions (sent on connect/reconnect)
    SyncSessions(Vec<SessionRecord>),
    /// Single session updated (includes stopped sessions)
    SessionUpdate(SessionRecord),
}

/// Manages upstream connection to host application.
pub struct UpstreamConnection {
    port: u16,
    container_id: String,
    container_name: String,
    working_dir: PathBuf,
}

impl UpstreamConnection {
    /// Create a new upstream connection manager.
    /// Reads container identification from environment variables.
    pub fn new(port: u16) -> Self {
        let container_id = std::env::var("PLANNING_AGENT_CONTAINER_ID")
            .unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().to_string());

        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        let container_name = std::env::var("PLANNING_AGENT_CONTAINER_NAME").unwrap_or_else(|_| {
            working_dir
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| container_id.clone())
        });

        Self {
            port,
            container_id,
            container_name,
            working_dir,
        }
    }

    /// Run upstream connection loop with automatic reconnection.
    /// This function runs indefinitely, reconnecting on disconnect.
    pub async fn run(self, mut session_rx: mpsc::UnboundedReceiver<UpstreamEvent>) {
        let mut attempt = 0u32;

        loop {
            match self.connect_and_run(&mut session_rx).await {
                Ok(()) => {
                    // Clean disconnect, reset backoff
                    attempt = 0;
                }
                Err(e) => {
                    eprintln!("[upstream] Connection error: {}", e);
                }
            }

            // Backoff before reconnect
            let delay = backoff_delay(attempt);
            attempt = attempt.saturating_add(1).min(10);
            eprintln!("[upstream] Reconnecting in {:?}...", delay);
            tokio::time::sleep(delay).await;
        }
    }

    async fn connect_and_run(
        &self,
        session_rx: &mut mpsc::UnboundedReceiver<UpstreamEvent>,
    ) -> anyhow::Result<()> {
        let stream = TcpStream::connect(format!("127.0.0.1:{}", self.port)).await?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send Hello
        let hello = DaemonToHost::Hello {
            container_id: self.container_id.clone(),
            container_name: self.container_name.clone(),
            working_dir: self.working_dir.clone(),
            protocol_version: PROTOCOL_VERSION,
        };
        let hello_json = serde_json::to_string(&hello)?;
        writer
            .write_all(format!("{}\n", hello_json).as_bytes())
            .await?;

        // Wait for Welcome
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let welcome: HostToDaemon = serde_json::from_str(line.trim())?;

        match welcome {
            HostToDaemon::Welcome {
                protocol_version, ..
            } => {
                if protocol_version != PROTOCOL_VERSION {
                    anyhow::bail!(
                        "Protocol version mismatch: got {}, expected {}",
                        protocol_version,
                        PROTOCOL_VERSION
                    );
                }
            }
            HostToDaemon::Ack => {
                anyhow::bail!("Expected Welcome, got Ack");
            }
        }

        eprintln!("[upstream] Connected to host on port {}", self.port);

        // Main loop: forward session events
        let heartbeat_interval = Duration::from_secs(30);
        let mut heartbeat_timer = tokio::time::interval(heartbeat_interval);

        loop {
            tokio::select! {
                event = session_rx.recv() => {
                    match event {
                        Some(UpstreamEvent::SyncSessions(records)) => {
                            let msg = DaemonToHost::SyncSessions {
                                sessions: records.iter().map(SessionInfo::from_session_record).collect(),
                            };
                            let json = serde_json::to_string(&msg)?;
                            writer.write_all(format!("{}\n", json).as_bytes()).await?;
                        }
                        Some(UpstreamEvent::SessionUpdate(record)) => {
                            let msg = DaemonToHost::SessionUpdate {
                                session: SessionInfo::from_session_record(&record),
                            };
                            let json = serde_json::to_string(&msg)?;
                            writer.write_all(format!("{}\n", json).as_bytes()).await?;
                        }
                        None => {
                            // Channel closed, exit
                            break;
                        }
                    }
                }
                _ = heartbeat_timer.tick() => {
                    let msg = DaemonToHost::Heartbeat;
                    let json = serde_json::to_string(&msg)?;
                    writer.write_all(format!("{}\n", json).as_bytes()).await?;
                }
                result = async {
                    let mut buf = String::new();
                    reader.read_line(&mut buf).await.map(|_| buf)
                } => {
                    match result {
                        Ok(buf) if buf.is_empty() => {
                            // EOF - server disconnected
                            break;
                        }
                        Ok(_) => {
                            // Received Ack or other message, ignore
                        }
                        Err(e) => {
                            return Err(e.into());
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Calculate backoff delay for reconnection attempt.
fn backoff_delay(attempt: u32) -> Duration {
    let base_secs = 5u64;
    let max_secs = 60u64;
    let delay_secs = (base_secs * 2u64.pow(attempt)).min(max_secs);
    Duration::from_secs(delay_secs)
}

/// Get the host port from environment or default.
pub fn host_port() -> Option<u16> {
    // Only enable upstream if PLANNING_AGENT_HOST_PORT is set
    // (to avoid connecting when not in a container)
    std::env::var("PLANNING_AGENT_HOST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_protocol::LivenessState;
    use crate::session_daemon::protocol::SessionRecord;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn test_backoff_delay() {
        assert_eq!(backoff_delay(0), Duration::from_secs(5));
        assert_eq!(backoff_delay(1), Duration::from_secs(10));
        assert_eq!(backoff_delay(2), Duration::from_secs(20));
        assert_eq!(backoff_delay(3), Duration::from_secs(40));
        assert_eq!(backoff_delay(4), Duration::from_secs(60)); // capped at 60
        assert_eq!(backoff_delay(5), Duration::from_secs(60)); // stays at 60
    }

    /// Create a test UpstreamConnection with custom settings.
    fn test_upstream(port: u16) -> UpstreamConnection {
        UpstreamConnection {
            port,
            container_id: "test-container".to_string(),
            container_name: "Test Container".to_string(),
            working_dir: PathBuf::from("/test"),
        }
    }

    /// Create a test SessionRecord.
    fn test_session_record(id: &str, feature: &str, status: &str) -> SessionRecord {
        SessionRecord {
            workflow_session_id: id.to_string(),
            feature_name: feature.to_string(),
            working_dir: PathBuf::from("/test"),
            state_path: PathBuf::from("/test/state.json"),
            phase: "Planning".to_string(),
            iteration: 1,
            workflow_status: status.to_string(),
            liveness: LivenessState::Running,
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_heartbeat_at: chrono::Utc::now().to_rfc3339(),
            pid: std::process::id(),
        }
    }

    #[tokio::test]
    async fn test_upstream_connection_flow() {
        // Start a mock host server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Channel for upstream events
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Spawn upstream connection task
        let upstream = test_upstream(port);
        let upstream_handle = tokio::spawn(async move {
            // connect_and_run returns when channel closes or server disconnects
            let mut rx = event_rx;
            let _ = upstream.connect_and_run(&mut rx).await;
        });

        // Accept connection as mock host
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Receive Hello
        reader.read_line(&mut line).await.unwrap();
        let hello: DaemonToHost = serde_json::from_str(line.trim()).unwrap();
        match hello {
            DaemonToHost::Hello {
                container_id,
                container_name,
                protocol_version,
                ..
            } => {
                assert_eq!(container_id, "test-container");
                assert_eq!(container_name, "Test Container");
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            _ => panic!("Expected Hello"),
        }
        line.clear();

        // Send Welcome
        let welcome = HostToDaemon::Welcome {
            host_version: "1.0.0".to_string(),
            protocol_version: PROTOCOL_VERSION,
        };
        writer
            .write_all(format!("{}\n", serde_json::to_string(&welcome).unwrap()).as_bytes())
            .await
            .unwrap();

        // Send SyncSessions event
        let sessions = vec![
            test_session_record("sess-1", "Feature A", "Running"),
            test_session_record("sess-2", "Feature B", "AwaitingApproval"),
        ];
        event_tx
            .send(UpstreamEvent::SyncSessions(sessions))
            .unwrap();

        // Receive SyncSessions message
        reader.read_line(&mut line).await.unwrap();
        let sync: DaemonToHost = serde_json::from_str(line.trim()).unwrap();
        match sync {
            DaemonToHost::SyncSessions { sessions } => {
                assert_eq!(sessions.len(), 2);
                assert_eq!(sessions[0].session_id, "sess-1");
                assert_eq!(sessions[1].session_id, "sess-2");
            }
            _ => panic!("Expected SyncSessions"),
        }
        line.clear();

        // Send Ack
        let ack = HostToDaemon::Ack;
        writer
            .write_all(format!("{}\n", serde_json::to_string(&ack).unwrap()).as_bytes())
            .await
            .unwrap();

        // Send SessionUpdate event
        let updated = test_session_record("sess-1", "Feature A", "Complete");
        event_tx
            .send(UpstreamEvent::SessionUpdate(updated))
            .unwrap();

        // Receive SessionUpdate message
        reader.read_line(&mut line).await.unwrap();
        let update: DaemonToHost = serde_json::from_str(line.trim()).unwrap();
        match update {
            DaemonToHost::SessionUpdate { session } => {
                assert_eq!(session.session_id, "sess-1");
                assert_eq!(session.status, "Complete");
            }
            _ => panic!("Expected SessionUpdate"),
        }

        // Close channel to signal shutdown
        drop(event_tx);

        // Wait for upstream to finish
        let _ = upstream_handle.await;
    }

    #[tokio::test]
    async fn test_upstream_protocol_version_mismatch() {
        // Start a mock host server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Channel for upstream events
        let (_event_tx, event_rx) = mpsc::unbounded_channel();

        // Spawn upstream connection task
        let upstream = test_upstream(port);
        let upstream_handle = tokio::spawn(async move {
            let mut rx = event_rx;
            upstream.connect_and_run(&mut rx).await
        });

        // Accept connection as mock host
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Receive Hello
        reader.read_line(&mut line).await.unwrap();

        // Send Welcome with wrong protocol version
        let welcome = HostToDaemon::Welcome {
            host_version: "1.0.0".to_string(),
            protocol_version: PROTOCOL_VERSION + 100, // Wrong version
        };
        writer
            .write_all(format!("{}\n", serde_json::to_string(&welcome).unwrap()).as_bytes())
            .await
            .unwrap();

        // Upstream should return an error
        let result = upstream_handle.await.unwrap();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Protocol version mismatch"));
    }
}
