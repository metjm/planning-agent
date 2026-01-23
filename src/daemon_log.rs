//! Shared debug logging utility for daemon components.

use std::io::Write;

/// Debug logging utility for daemon components.
///
/// The `tag` parameter identifies the source module (e.g., "server", "upstream",
/// "subscription", "tui_runner") to aid debugging.
///
/// Writes to ~/.planning-agent/daemon-debug.log
pub fn daemon_log(tag: &str, msg: &str) {
    if let Ok(home) = crate::planning_paths::planning_agent_home_dir() {
        let log_path = home.join("daemon-debug.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(file, "[{}] [{}] {}", now, tag, msg);
        }
    }
}
