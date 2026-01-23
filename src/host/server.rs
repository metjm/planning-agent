//! TCP server for host mode.
//!
//! Accepts connections from container daemons and aggregates their session data.
//! Uses newline-delimited JSON protocol matching the upstream connection format.

use crate::host::state::HostState;
use crate::host_protocol::{DaemonToHost, HostToDaemon, PROTOCOL_VERSION};
#[cfg(feature = "host-gui")]
use anyhow::Context;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(feature = "host-gui")]
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};

/// Events sent from TCP server to GUI.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// A new container daemon connected.
    ContainerConnected {
        container_id: String,
        container_name: String,
    },
    /// A container daemon disconnected.
    ContainerDisconnected { container_id: String },
    /// Sessions were updated (sync, update, or removal).
    SessionsUpdated,
}

/// Run the host TCP server.
#[cfg(feature = "host-gui")]
pub async fn run_server(
    port: u16,
    state: Arc<Mutex<HostState>>,
    event_tx: mpsc::UnboundedSender<HostEvent>,
) -> Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .with_context(|| format!("Failed to bind to port {}", port))?;

    eprintln!("[host] Listening on 0.0.0.0:{}", port);

    loop {
        let (stream, addr) = listener.accept().await?;
        eprintln!("[host] Connection from {}", addr);

        let conn_state = state.clone();
        let conn_event_tx = event_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, conn_state, conn_event_tx).await {
                eprintln!("[host] Connection error: {}", e);
            }
        });
    }
}

/// Handles a single container daemon connection.
/// Made pub(crate) for testing.
pub(crate) async fn handle_connection(
    stream: TcpStream,
    state: Arc<Mutex<HostState>>,
    event_tx: mpsc::UnboundedSender<HostEvent>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Wait for Hello message
    reader.read_line(&mut line).await?;
    let hello: DaemonToHost = serde_json::from_str(line.trim())?;

    let container_id = match hello {
        DaemonToHost::Hello {
            container_id,
            container_name,
            working_dir,
            protocol_version,
        } => {
            eprintln!(
                "[host] Received Hello from container_id={}, name={}, working_dir={:?}, protocol_version={}",
                container_id, container_name, working_dir, protocol_version
            );

            // Check protocol version
            if protocol_version != PROTOCOL_VERSION {
                eprintln!(
                    "[host] Protocol version mismatch: got {}, expected {}",
                    protocol_version, PROTOCOL_VERSION
                );
                return Ok(());
            }

            // Register container
            {
                let mut state_guard = state.lock().await;
                state_guard.add_container(container_id.clone(), container_name.clone());
                eprintln!(
                    "[host] Registered container, total containers: {}",
                    state_guard.containers.len()
                );
            }

            // Send Welcome
            let welcome = HostToDaemon::Welcome {
                host_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION,
            };
            let welcome_json = serde_json::to_string(&welcome)?;
            eprintln!("[host] Sending Welcome: {}", welcome_json);
            writer
                .write_all(format!("{}\n", welcome_json).as_bytes())
                .await?;

            // Notify GUI
            let _ = event_tx.send(HostEvent::ContainerConnected {
                container_id: container_id.clone(),
                container_name,
            });

            eprintln!("[host] Handshake complete for container {}", container_id);
            container_id
        }
        _ => {
            eprintln!("[host] Expected Hello, got {:?}", hello);
            return Ok(());
        }
    };

    // Handle ongoing messages
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let message: DaemonToHost = match serde_json::from_str(line.trim()) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("[host] Invalid message: {}", e);
                        continue;
                    }
                };

                match message {
                    DaemonToHost::SyncSessions { sessions } => {
                        eprintln!(
                            "[host] Received SyncSessions from {} with {} sessions",
                            container_id,
                            sessions.len()
                        );
                        for (i, s) in sessions.iter().enumerate() {
                            eprintln!(
                                "[host]   Session {}: id={}, feature={}, phase={}, status={}",
                                i, s.session_id, s.feature_name, s.phase, s.status
                            );
                        }
                        let mut state_guard = state.lock().await;
                        state_guard.sync_sessions(&container_id, sessions);
                        let _ = event_tx.send(HostEvent::SessionsUpdated);
                        eprintln!("[host] SyncSessions processed, notified GUI");
                    }
                    DaemonToHost::SessionUpdate { session } => {
                        eprintln!(
                            "[host] Received SessionUpdate from {}: id={}, feature={}, phase={}, status={}",
                            container_id, session.session_id, session.feature_name, session.phase, session.status
                        );
                        let mut state_guard = state.lock().await;
                        state_guard.update_session(&container_id, session);
                        let _ = event_tx.send(HostEvent::SessionsUpdated);
                        eprintln!("[host] SessionUpdate processed, notified GUI");
                    }
                    DaemonToHost::SessionGone { session_id } => {
                        eprintln!(
                            "[host] Received SessionGone from {}: session_id={}",
                            container_id, session_id
                        );
                        let mut state_guard = state.lock().await;
                        state_guard.remove_session(&container_id, &session_id);
                        let _ = event_tx.send(HostEvent::SessionsUpdated);
                    }
                    DaemonToHost::Heartbeat => {
                        eprintln!("[host] Received Heartbeat from {}", container_id);
                        let mut state_guard = state.lock().await;
                        state_guard.heartbeat(&container_id);
                    }
                    DaemonToHost::Hello { .. } => {
                        eprintln!("[host] Unexpected Hello after handshake");
                    }
                }

                // Send Ack
                let ack = HostToDaemon::Ack;
                let ack_json = serde_json::to_string(&ack)?;
                writer
                    .write_all(format!("{}\n", ack_json).as_bytes())
                    .await?;
            }
            Err(e) => {
                eprintln!("[host] Read error: {}", e);
                break;
            }
        }
    }

    // Cleanup on disconnect
    {
        let mut state_guard = state.lock().await;
        state_guard.remove_container(&container_id);
    }
    let _ = event_tx.send(HostEvent::ContainerDisconnected {
        container_id: container_id.clone(),
    });
    eprintln!("[host] Container {} disconnected", container_id);

    Ok(())
}
