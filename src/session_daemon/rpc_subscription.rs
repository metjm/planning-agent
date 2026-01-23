//! RPC subscription for receiving push notifications from daemon.
//!
//! This module provides `RpcSubscription` which connects to the daemon's
//! subscriber port and receives push notifications via tarpc callbacks.

use crate::daemon_log::daemon_log;
use crate::planning_paths;
use crate::rpc::daemon_service::{DaemonServiceClient, SubscriberCallback};
use crate::rpc::{PortFileContent, SessionRecord};
use futures::StreamExt;
use tarpc::server::{self, Channel};
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::mpsc;

/// Events received from daemon via subscription.
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    /// A session was created, updated, or changed liveness state
    SessionChanged(Box<SessionRecord>),
    /// Daemon is restarting (subscriber should reconnect)
    DaemonRestarting,
}

/// Handler that implements SubscriberCallback and forwards events to a channel.
#[derive(Clone)]
struct SubscriptionHandler {
    tx: mpsc::UnboundedSender<SubscriptionEvent>,
}

impl SubscriberCallback for SubscriptionHandler {
    async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
        let _ = self
            .tx
            .send(SubscriptionEvent::SessionChanged(Box::new(record)));
    }

    async fn daemon_restarting(self, _: tarpc::context::Context, new_sha: String) {
        daemon_log(
            "rpc_subscription",
            &format!("Daemon restarting notification: {}", new_sha),
        );
        let _ = self.tx.send(SubscriptionEvent::DaemonRestarting);
    }

    async fn ping(self, _: tarpc::context::Context) -> bool {
        true
    }
}

/// Async subscription that receives push notifications from daemon via tarpc.
///
/// Architecture:
/// 1. Subscriber reads port file to get main port, subscriber port, and auth token
/// 2. Subscriber connects to main RPC port and authenticates
/// 3. Subscriber connects to subscriber port
/// 4. Subscriber runs a SubscriberCallback RPC server on that connection
/// 5. Daemon calls into the subscriber's server to push notifications
pub struct RpcSubscription {
    rx: mpsc::UnboundedReceiver<SubscriptionEvent>,
    _server_task: tokio::task::JoinHandle<()>,
}

impl RpcSubscription {
    /// Connect to daemon's subscriber port and establish subscription.
    ///
    /// Returns None if unable to connect (daemon not running or connection failed).
    pub async fn connect() -> Option<Self> {
        use tarpc::serde_transport::tcp;
        use tarpc::{client, context};

        daemon_log("rpc_subscription", "connect() called");

        // Read port file to get ports and auth token
        let port_path = planning_paths::sessiond_port_path().ok()?;
        let content = std::fs::read_to_string(&port_path).ok()?;
        let port_info: PortFileContent = serde_json::from_str(&content).ok()?;

        daemon_log(
            "rpc_subscription",
            &format!(
                "Read port file: main={}, subscriber={}",
                port_info.port, port_info.subscriber_port
            ),
        );

        // First, authenticate via main RPC to establish trust
        let main_addr = format!("127.0.0.1:{}", port_info.port);
        let auth_transport = match tcp::connect(&main_addr, Bincode::default).await {
            Ok(t) => t,
            Err(e) => {
                daemon_log(
                    "rpc_subscription",
                    &format!("Failed to connect to main RPC: {}", e),
                );
                return None;
            }
        };

        let auth_client =
            DaemonServiceClient::new(client::Config::default(), auth_transport).spawn();

        // Authenticate - this validates we have the correct token
        match auth_client
            .authenticate(context::current(), port_info.token.clone())
            .await
        {
            Ok(Ok(())) => {
                daemon_log("rpc_subscription", "Authenticated successfully");
            }
            Ok(Err(e)) => {
                daemon_log(
                    "rpc_subscription",
                    &format!("Authentication rejected: {}", e),
                );
                return None;
            }
            Err(e) => {
                daemon_log(
                    "rpc_subscription",
                    &format!("Authentication RPC error: {}", e),
                );
                return None;
            }
        }

        // Now connect to subscriber port
        let subscriber_addr = format!("127.0.0.1:{}", port_info.subscriber_port);
        let transport = match tcp::connect(&subscriber_addr, Bincode::default).await {
            Ok(t) => t,
            Err(e) => {
                daemon_log(
                    "rpc_subscription",
                    &format!("Failed to connect to subscriber port: {}", e),
                );
                return None;
            }
        };

        daemon_log(
            "rpc_subscription",
            &format!("Connected to subscriber port {}", port_info.subscriber_port),
        );

        // Create channel for forwarding events to caller
        let (tx, rx) = mpsc::unbounded_channel();
        let handler = SubscriptionHandler { tx };

        // Run a SubscriberCallback server on our end of the connection.
        // The daemon will call into this server to push notifications.
        let server_task = tokio::spawn(async move {
            daemon_log("rpc_subscription", "Starting callback server");
            let channel = server::BaseChannel::with_defaults(transport);
            channel
                .execute(handler.serve())
                .for_each(|response| async {
                    tokio::spawn(response);
                })
                .await;
            daemon_log("rpc_subscription", "Callback server ended");
        });

        daemon_log("rpc_subscription", "Subscription connected successfully");

        Some(Self {
            rx,
            _server_task: server_task,
        })
    }

    /// Receive the next subscription event.
    /// Returns None if the connection is closed.
    pub async fn recv(&mut self) -> Option<SubscriptionEvent> {
        self.rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_event_debug() {
        let record = SessionRecord::new(
            "test".to_string(),
            "Test".to_string(),
            std::path::PathBuf::from("/test"),
            std::path::PathBuf::from("/test/state.json"),
            "Planning".to_string(),
            1,
            "Planning".to_string(),
            12345,
        );

        let event = SubscriptionEvent::SessionChanged(Box::new(record));
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("SessionChanged"));

        let event = SubscriptionEvent::DaemonRestarting;
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("DaemonRestarting"));
    }
}
