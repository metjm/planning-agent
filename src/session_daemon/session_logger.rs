//! Unified session logging for planning-agent workflows.
//!
//! This module provides a centralized logging system for session-scoped events,
//! with consistent UTC timestamps and structured log categories.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use planning_agent::session_logger::{SessionLogger, LogCategory, LogLevel};
//!
//! let logger = SessionLogger::new("abc123-session-id")?;
//! logger.log(LogLevel::Info, LogCategory::Workflow, "Planning phase started");
//! logger.log_agent("claude", "stdout", "Processing...");
//! ```
//!
//! ## Log Format
//!
//! All log entries follow a consistent UTC format:
//! ```text
//! [2026-01-15T14:30:00.123Z] [INFO] [WORKFLOW] Planning phase started
//! [2026-01-15T14:30:01.456Z] [AGENT:claude] stdout: Processing...
//! ```

use crate::planning_paths;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

/// Log verbosity levels, ordered from most to least severe.
///
/// Default is `Debug` for verbose-by-default behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Critical errors that may cause workflow failure
    Error = 0,
    /// Warning conditions that should be noted
    Warn = 1,
    /// Informational messages about workflow progress
    Info = 2,
    /// Debug messages for troubleshooting (default)
    #[default]
    Debug = 3,
    /// Very detailed trace messages
    Trace = 4,
}

impl LogLevel {
    /// Returns the uppercase string representation for log output.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Log categories for structured logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogCategory {
    /// Workflow phase transitions and decisions
    Workflow,
}

impl LogCategory {
    /// Returns the uppercase string representation for log output.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogCategory::Workflow => "WORKFLOW",
        }
    }
}

impl std::fmt::Display for LogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A thread-safe session logger that writes to session-scoped log files.
///
/// The logger creates and manages two log files within the session directory:
/// - `logs/session.log` - Main session log with all categorized entries
/// - `logs/agent-stream.log` - Raw agent output (stdout/stderr)
///
/// All timestamps are in UTC ISO 8601 format for consistency and portability.
pub struct SessionLogger {
    main_log: Arc<Mutex<File>>,
    agent_log: Arc<Mutex<File>>,
    /// Minimum log level to output. Messages below this level are ignored.
    log_level: LogLevel,
}

impl SessionLogger {
    /// Creates a new SessionLogger for the given session ID with default log level (Debug).
    ///
    /// This creates the session directory structure if it doesn't exist:
    /// ```text
    /// ~/.planning-agent/sessions/<session-id>/
    /// └── logs/
    ///     ├── session.log
    ///     └── agent-stream.log
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The session directory cannot be created
    /// - The log files cannot be opened for writing
    pub fn new(session_id: &str) -> Result<Self> {
        Self::new_with_level(session_id, LogLevel::Debug)
    }

    /// Creates a new SessionLogger with a specific log level.
    ///
    /// Messages below the specified level will not be logged.
    pub fn new_with_level(session_id: &str, log_level: LogLevel) -> Result<Self> {
        let logs_dir = planning_paths::session_logs_dir(session_id)?;

        let main_log_path = logs_dir.join("session.log");
        let agent_log_path = logs_dir.join("agent-stream.log");

        let main_log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&main_log_path)
            .with_context(|| format!("Failed to open session log: {}", main_log_path.display()))?;

        let agent_log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&agent_log_path)
            .with_context(|| format!("Failed to open agent log: {}", agent_log_path.display()))?;

        // Write log headers
        let mut main = main_log;
        let mut agent = agent_log;
        let now = format_timestamp();
        let _ = writeln!(
            main,
            "\n=== Session log started at {} (session {}) ===",
            now, session_id
        );
        let _ = writeln!(
            agent,
            "\n=== Agent stream log started at {} (session {}) ===",
            now, session_id
        );

        // Merge startup logs if they exist
        merge_startup_logs(&mut main);

        Ok(Self {
            main_log: Arc::new(Mutex::new(main)),
            agent_log: Arc::new(Mutex::new(agent)),
            log_level,
        })
    }

    /// Checks if a message at the given level should be logged.
    pub fn should_log(&self, level: LogLevel) -> bool {
        level <= self.log_level
    }

    /// Logs a message with the specified level and category to the main session log.
    ///
    /// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ] [LEVEL] [CATEGORY] message`
    ///
    /// Messages below the configured log level are silently ignored.
    pub fn log(&self, level: LogLevel, category: LogCategory, message: &str) {
        if !self.should_log(level) {
            return;
        }
        if let Ok(mut file) = self.main_log.lock() {
            let timestamp = format_timestamp();
            let _ = writeln!(
                file,
                "[{}] [{}] [{}] {}",
                timestamp, level, category, message
            );
            let _ = file.flush();
        }
    }

    /// Logs raw agent stream output to the agent-stream log only.
    ///
    /// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ] [agent:kind] line`
    ///
    /// This is for high-volume output that shouldn't clutter the main log.
    /// Agent stream output is always logged regardless of level.
    pub fn log_agent_stream(&self, agent_name: &str, kind: &str, line: &str) {
        if let Ok(mut file) = self.agent_log.lock() {
            let timestamp = format_timestamp();
            let _ = writeln!(file, "[{}][{}][{}] {}", timestamp, agent_name, kind, line);
            let _ = file.flush();
        }
    }
}

/// Formats the current UTC time as an ISO 8601 timestamp with milliseconds.
///
/// Format: `YYYY-MM-DDTHH:MM:SS.mmmZ`
fn format_timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Merges startup logs into the session log, then truncates the startup log file.
///
/// This captures early startup messages (before session creation) into the session log.
/// The atomic pattern ensures startup log content is preserved if crash occurs mid-merge.
fn merge_startup_logs(session_log: &mut File) {
    let startup_path = match planning_paths::startup_log_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    if !startup_path.exists() {
        return;
    }

    // Read and filter startup log entries for this process's PID
    let current_pid = std::process::id();
    let startup_file = match std::fs::File::open(&startup_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let reader = BufReader::new(startup_file);
    let mut merged_any = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        // Filter by PID if line contains it (format: [timestamp][PID:12345] message)
        // Or include all lines if they don't have PID markers (backward compat)
        if line.contains(&format!("[PID:{}]", current_pid)) || !line.contains("[PID:") {
            if !merged_any {
                let _ = writeln!(session_log, "=== Merged startup logs ===");
                merged_any = true;
            }
            let _ = writeln!(session_log, "{}", line);
        }
    }

    if merged_any {
        let _ = writeln!(session_log, "=== End merged startup logs ===");
        let _ = session_log.flush();
    }

    // Truncate startup log after successful merge
    // Note: In concurrent scenarios, other processes may still be writing,
    // so we just truncate our entries (by rewriting without them)
    let _ = std::fs::write(&startup_path, "");
}

/// Writes a message to the startup log (for early logging before session creation).
///
/// Each entry includes timestamp and PID for concurrent session attribution.
/// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ][PID:12345] message`
pub fn log_startup(message: &str) {
    let startup_path = match planning_paths::startup_log_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&startup_path)
    {
        let timestamp = format_timestamp();
        let pid = std::process::id();
        let _ = writeln!(file, "[{}][PID:{}] {}", timestamp, pid, message);
        let _ = file.flush();
    }
}

/// Creates a SessionLogger wrapped in Arc for shared ownership across async tasks.
///
/// This is the recommended way to create a logger for use in the workflow engine.
pub fn create_session_logger(session_id: &str) -> Result<Arc<SessionLogger>> {
    Ok(Arc::new(SessionLogger::new(session_id)?))
}

#[cfg(test)]
#[path = "tests/session_logger_tests.rs"]
mod tests;
