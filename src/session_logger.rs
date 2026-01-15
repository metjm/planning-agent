//! Unified session logging for planning-agent workflows.
//!
//! This module provides a centralized logging system for session-scoped events,
//! with consistent UTC timestamps and structured log categories.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use planning_agent::session_logger::{SessionLogger, LogCategory};
//!
//! let logger = SessionLogger::new("abc123-session-id")?;
//! logger.log(LogCategory::Workflow, "Planning phase started");
//! logger.log_agent("claude", "stdout", "Processing...");
//! ```
//!
//! ## Log Format
//!
//! All log entries follow a consistent UTC format:
//! ```text
//! [2026-01-15T14:30:00.123Z] [WORKFLOW] Planning phase started
//! [2026-01-15T14:30:01.456Z] [AGENT:claude] stdout: Processing...
//! ```

use crate::planning_paths;
use anyhow::{Context, Result};
use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
#[allow(dead_code)]
pub struct SessionLogger {
    session_id: String,
    session_dir: PathBuf,
    main_log: Arc<Mutex<File>>,
    agent_log: Arc<Mutex<File>>,
}

#[allow(dead_code)]
impl SessionLogger {
    /// Creates a new SessionLogger for the given session ID.
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

        Ok(Self {
            session_id: session_id.to_string(),
            session_dir,
            main_log: Arc::new(Mutex::new(main)),
            agent_log: Arc::new(Mutex::new(agent)),
        })
    }

    /// Returns the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the session directory path.
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Logs a message with the specified category to the main session log.
    ///
    /// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ] [CATEGORY] message`
    pub fn log(&self, category: LogCategory, message: &str) {
        if let Ok(mut file) = self.main_log.lock() {
            let timestamp = format_timestamp();
            let _ = writeln!(file, "[{}] [{}] {}", timestamp, category, message);
            let _ = file.flush();
        }
    }

    /// Logs an agent-related message with agent name and kind.
    ///
    /// Format: `[YYYY-MM-DDTHH:MM:SS.mmmZ] [AGENT:name] kind: message`
    ///
    /// This writes to both the main session log and the agent-stream log.
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
    pub fn log_agent_stream(&self, agent_name: &str, kind: &str, line: &str) {
        if let Ok(mut file) = self.agent_log.lock() {
            let timestamp = format_timestamp();
            let _ = writeln!(file, "[{}][{}][{}] {}", timestamp, agent_name, kind, line);
            let _ = file.flush();
        }
    }

    /// Logs an info-level message.
    pub fn info(&self, message: &str) {
        self.log(LogCategory::System, &format!("INFO: {}", message));
    }

    /// Logs a warning-level message.
    pub fn warn(&self, message: &str) {
        self.log(LogCategory::System, &format!("WARN: {}", message));
    }

    /// Logs an error-level message.
    pub fn error(&self, message: &str) {
        self.log(LogCategory::System, &format!("ERROR: {}", message));
    }

    /// Logs a debug-level message.
    pub fn debug(&self, message: &str) {
        self.log(LogCategory::System, &format!("DEBUG: {}", message));
    }

    /// Logs a workflow phase transition.
    pub fn log_phase_transition(&self, from: &str, to: &str) {
        self.log(
            LogCategory::Workflow,
            &format!("Phase transition: {} -> {}", from, to),
        );
    }

    /// Logs a state save event.
    pub fn log_state_save(&self, path: &Path) {
        self.log(
            LogCategory::State,
            &format!("State saved to: {}", path.display()),
        );
    }

    /// Logs a state load event.
    pub fn log_state_load(&self, path: &Path) {
        self.log(
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

/// Creates a SessionLogger wrapped in Arc for shared ownership across async tasks.
///
/// This is the recommended way to create a logger for use in the workflow engine.
#[allow(dead_code)]
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
        logger.log(LogCategory::Workflow, "Test workflow message");
        logger.log_agent("test-agent", "stdout", "Test agent output");
        logger.log_agent_stream("test-agent", "stderr", "Test stream output");
        logger.info("Test info");
        logger.warn("Test warning");
        logger.error("Test error");
        logger.debug("Test debug");
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
}
