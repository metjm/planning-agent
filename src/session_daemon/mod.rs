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
//! - **Daemon (`server.rs`)**: Unix socket (or TCP on Windows) server that maintains
//!   an in-memory registry of active sessions.
//! - **Client (`client.rs`)**: Connect-or-spawn client that registers sessions and
//!   sends heartbeats.
//! - **Protocol (`protocol.rs`)**: Newline-delimited JSON messages for IPC.

pub mod client;
pub mod protocol;
pub mod server;
pub mod subscription;

#[cfg(test)]
mod server_tests;

#[cfg(test)]
#[cfg(unix)]
mod client_tests;

#[cfg(test)]
#[cfg(unix)]
mod subscription_tests;

pub use client::SessionDaemonClient;
pub use protocol::{LivenessState, SessionRecord};
pub use server::run_daemon;
#[allow(unused_imports)]
pub use subscription::DaemonSubscription;
