//! Host mode for aggregating sessions from multiple containers.
//!
//! This module provides a native desktop GUI (using egui/eframe) that:
//! - Runs a TCP server accepting connections from container daemons
//! - Shows all sessions from all containers in a dashboard
//! - Provides a macOS menu bar icon with session count
//! - Real-time updates using egui's immediate mode rendering
//!
//! The GUI components require the `host-gui` feature to be enabled.
//! The server and state modules are compiled when testing or when host-gui is enabled.

#[cfg(feature = "host-gui")]
pub mod gui;

#[cfg(any(feature = "host-gui", test))]
pub mod rpc_server;
#[cfg(any(feature = "host-gui", test))]
pub mod server;
#[cfg(any(feature = "host-gui", test))]
pub mod state;

#[cfg(test)]
mod server_tests;
