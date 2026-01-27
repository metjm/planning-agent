//! Session management daemon for cross-process session registry.
//!
//! This module provides a lightweight daemon that tracks all running planning-agent
//! sessions across multiple processes, enabling:
//! - Live session status updates across processes
//! - Session resume from the `/sessions` overlay
//! - Coordinated daemon restarts on `/update`
//!
//! ## Architecture
//!
//! - **Daemon (`rpc_server.rs`)**: tarpc-based RPC server that maintains
//!   an in-memory registry of active sessions.
//! - **Client (`rpc_client.rs`)**: Connect-or-spawn client that registers sessions and
//!   sends heartbeats using tarpc RPC.
//! - **Subscription (`rpc_subscription.rs`)**: tarpc-based push notification subscriber.
//! - **Protocol (`protocol.rs`)**: Message types and session records.

pub mod file_service_impl;
pub mod protocol;
pub mod rpc_client;
pub mod rpc_server;
pub mod rpc_subscription;
pub mod rpc_upstream;
pub mod server;
pub mod session_logger;
pub mod session_store;
pub mod session_tracking;

#[cfg(test)]
#[path = "tests/server_tests.rs"]
mod server_tests;

#[cfg(test)]
pub(crate) mod rpc_tests;

pub use protocol::{LivenessState, SessionRecord};
pub use rpc_client::RpcClient;
pub use rpc_server::run_daemon_rpc;
pub use session_logger::*;
pub use session_store::*;
pub use session_tracking::*;

// Re-export WorkflowEventEnvelope for session tracking
pub use crate::rpc::WorkflowEventEnvelope;
