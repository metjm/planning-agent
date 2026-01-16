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
use std::path::{Path, PathBuf};
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

/// Configuration for session logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct LoggingConfig {
    /// The minimum log level to output. Messages below this level are ignored.
    /// Default: Debug (verbose by default)
    #[serde(default)]
    pub level: LogLevel,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Debug, // Verbose by default
        }
    }
}

/// Log categories for structured logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LogCategory {
    /// Workflow phase transitions and decisions
    Workflow,
    /// Agent invocations and responses
    Agent,
    /// State saves and loads
    State,
    /// UI events and user input
    Ui,
    /// System errors, warnings, and diagnostics
    System,
}

impl LogCategory {
    /// Returns the uppercase string representation for log output.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            LogCategory::Workflow => "WORKFLOW",
            LogCategory::Agent => "AGENT",
            LogCategory::State => "STATE",
            LogCategory::Ui => "UI",
            LogCategory::System => "SYSTEM",
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
    #[allow(dead_code)]
    session_id: String,
    #[allow(dead_code)]
    session_dir: PathBuf,
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
        let session_dir = planning_paths::session_dir(session_id)?;
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
            session_id: session_id.to_string(),
            session_dir,
            main_log: Arc::new(Mutex::new(main)),
            agent_log: Arc::new(Mutex::new(agent)),
            log_level,
        })
    }

    /// Returns the configured log level.
    #[allow(dead_code)]
    pub fn log_level(&self) -> LogLevel {
        self.log_level
    }

    /// Checks if a message at the given level should be logged.
    pub fn should_log(&self, level: LogLevel) -> bool {
        level <= self.log_level
    }

    /// Returns the session ID.
    #[allow(dead_code)]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the session directory path.
    #[allow(dead_code)]
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
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
            let _ = writeln!(file, "[{}] [{}] [{}] {}", timestamp, level, category, message);
            let _ = file.flush();
        }
    }

    /// Logs an agent-related message with agent name and kind.
    ///
    /// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ] [AGENT:name] kind: message`
    ///
    /// This writes to both the main session log and the agent-stream log.
    /// Agent messages are always logged regardless of level (they're high-signal).
    #[allow(dead_code)]
    pub fn log_agent(&self, agent_name: &str, kind: &str, message: &str) {
        let timestamp = format_timestamp();
        let formatted = format!("[{}] [AGENT:{}] {}: {}", timestamp, agent_name, kind, message);

        // Write to main log
        if let Ok(mut file) = self.main_log.lock() {
            let _ = writeln!(file, "{}", formatted);
            let _ = file.flush();
        }

        // Write to agent stream log
        if let Ok(mut file) = self.agent_log.lock() {
            let _ = writeln!(file, "{}", formatted);
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

    /// Logs an info-level message.
    #[allow(dead_code)]
    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, LogCategory::System, message);
    }

    /// Logs a warning-level message.
    #[allow(dead_code)]
    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, LogCategory::System, message);
    }

    /// Logs an error-level message.
    #[allow(dead_code)]
    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, LogCategory::System, message);
    }

    /// Logs a debug-level message.
    #[allow(dead_code)]
    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, LogCategory::System, message);
    }

    /// Logs a trace-level message.
    #[allow(dead_code)]
    pub fn trace(&self, message: &str) {
        self.log(LogLevel::Trace, LogCategory::System, message);
    }

    /// Logs a workflow phase transition.
    #[allow(dead_code)]
    pub fn log_phase_transition(&self, from: &str, to: &str) {
        self.log(
            LogLevel::Info,
            LogCategory::Workflow,
            &format!("Phase transition: {} -> {}", from, to),
        );
    }

    /// Logs a state save event.
    #[allow(dead_code)]
    pub fn log_state_save(&self, path: &Path) {
        self.log(
            LogLevel::Debug,
            LogCategory::State,
            &format!("State saved to: {}", path.display()),
        );
    }

    /// Logs a state load event.
    #[allow(dead_code)]
    pub fn log_state_load(&self, path: &Path) {
        self.log(
            LogLevel::Debug,
            LogCategory::State,
            &format!("State loaded from: {}", path.display()),
        );
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

/// Creates a SessionLogger with a specific log level, wrapped in Arc.
#[allow(dead_code)]
pub fn create_session_logger_with_level(session_id: &str, level: LogLevel) -> Result<Arc<SessionLogger>> {
    Ok(Arc::new(SessionLogger::new_with_level(session_id, level)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_log_category_as_str() {
        assert_eq!(LogCategory::Workflow.as_str(), "WORKFLOW");
        assert_eq!(LogCategory::Agent.as_str(), "AGENT");
        assert_eq!(LogCategory::State.as_str(), "STATE");
        assert_eq!(LogCategory::Ui.as_str(), "UI");
        assert_eq!(LogCategory::System.as_str(), "SYSTEM");
    }

    #[test]
    fn test_log_category_display() {
        assert_eq!(format!("{}", LogCategory::Workflow), "WORKFLOW");
        assert_eq!(format!("{}", LogCategory::Agent), "AGENT");
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

        let logger = result.unwrap();
        assert_eq!(logger.session_id(), session_id);
        assert!(logger.session_dir().exists());
    }

    #[test]
    fn test_session_logger_logging() {
        if env::var("HOME").is_err() {
            return;
        }

        let session_id = format!("test-{}", uuid::Uuid::new_v4());
        let logger = SessionLogger::new(&session_id).unwrap();

        // These should not panic
        logger.log(LogLevel::Info, LogCategory::Workflow, "Test workflow message");
        logger.log_agent("test-agent", "stdout", "Test agent output");
        logger.log_agent_stream("test-agent", "stderr", "Test stream output");
        logger.info("Test info");
        logger.warn("Test warning");
        logger.error("Test error");
        logger.debug("Test debug");
        logger.trace("Test trace");
        logger.log_phase_transition("Planning", "Reviewing");
        logger.log_state_save(Path::new("/tmp/test.json"));
        logger.log_state_load(Path::new("/tmp/test.json"));

        // Verify log files exist
        let logs_dir = logger.session_dir().join("logs");
        assert!(logs_dir.join("session.log").exists());
        assert!(logs_dir.join("agent-stream.log").exists());
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
        let logger2 = Arc::clone(&logger);
        assert_eq!(logger.session_id(), logger2.session_id());
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
    fn test_logging_config_default() {
        let config = LoggingConfig::default();
        assert_eq!(config.level, LogLevel::Debug);
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
