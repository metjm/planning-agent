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

pub mod protocol;
pub mod rpc_client;
pub mod rpc_server;
pub mod rpc_subscription;
pub mod rpc_upstream;
pub mod server;

#[cfg(test)]
mod server_tests;

#[cfg(test)]
mod rpc_tests;

pub use protocol::{LivenessState, SessionRecord};
pub use rpc_client::RpcClient;
pub use rpc_server::run_daemon_rpc;
