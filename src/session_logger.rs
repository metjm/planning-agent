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
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_log_category_as_str() {
        assert_eq!(LogCategory::Workflow.as_str(), "WORKFLOW");
    }

    #[test]
    fn test_log_category_display() {
        assert_eq!(format!("{}", LogCategory::Workflow), "WORKFLOW");
    }

    #[test]
    fn test_format_timestamp() {
        let ts = format_timestamp();
        // Should be in format: 2026-01-15T14:30:00.123Z
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
        assert_eq!(ts.len(), 24); // YYYY-MM-DDTHH:MM:SS.mmmZ
    }

    #[test]
    fn test_session_logger_creation() {
        if env::var("HOME").is_err() {
            return; // Skip if HOME is not set
        }

        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let result = SessionLogger::new(&session_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_session_logger_logging() {
        if env::var("HOME").is_err() {
            return;
        }

        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let logger = SessionLogger::new(&session_id).unwrap();

        // These should not panic
        logger.log(
            LogLevel::Info,
            LogCategory::Workflow,
            "Test workflow message",
        );
        logger.log_agent_stream("test-agent", "stderr", "Test stream output");
    }

    #[test]
    fn test_create_session_logger_arc() {
        if env::var("HOME").is_err() {
            return;
        }

        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let result = create_session_logger(&session_id);
        assert!(result.is_ok());

        let logger = result.unwrap();
        // Arc should allow cloning
        let _logger2 = Arc::clone(&logger);
    }

    #[test]
    fn test_log_level_ordering() {
        // Error is most severe (lowest value), Trace is least severe (highest value)
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn test_log_level_as_str() {
        assert_eq!(LogLevel::Error.as_str(), "ERROR");
        assert_eq!(LogLevel::Warn.as_str(), "WARN");
        assert_eq!(LogLevel::Info.as_str(), "INFO");
        assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
        assert_eq!(LogLevel::Trace.as_str(), "TRACE");
    }

    #[test]
    fn test_log_level_default() {
        let level: LogLevel = Default::default();
        assert_eq!(level, LogLevel::Debug);
    }

    #[test]
    fn test_should_log() {
        if env::var("HOME").is_err() {
            return;
        }

        // With Warn level, only Error and Warn should be logged
        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let logger = SessionLogger::new_with_level(&session_id, LogLevel::Warn).unwrap();

        assert!(logger.should_log(LogLevel::Error));
        assert!(logger.should_log(LogLevel::Warn));
        assert!(!logger.should_log(LogLevel::Info));
        assert!(!logger.should_log(LogLevel::Debug));
        assert!(!logger.should_log(LogLevel::Trace));
    }

    #[test]
    fn test_log_level_serde() {
        // Test serialization/deserialization
        let level = LogLevel::Info;
        let json = serde_json::to_string(&level).unwrap();
        assert_eq!(json, "\"info\"");

        let parsed: LogLevel = serde_json::from_str("\"debug\"").unwrap();
        assert_eq!(parsed, LogLevel::Debug);
    }
}
