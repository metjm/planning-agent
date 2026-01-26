//! Structured JSONL logger for debugging and event reconstruction.
//!
//! This module provides machine-parseable logging with:
//! - Monotonic sequence numbers for ordering
//! - ISO 8601 timestamps with microsecond precision
//! - Session and run IDs for correlation
//! - Structured event data in JSON format

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::domain::WorkflowCommand;
use crate::domain::WorkflowEvent;

/// Structured JSONL logger for debugging and event reconstruction.
pub struct StructuredLogger {
    session_id: String,
    run_id: AtomicU64,
    seq: AtomicU64,
    log_file: Mutex<File>,
    log_path: PathBuf,
}

/// A single log entry in JSONL format.
#[derive(Serialize, serde::Deserialize)]
pub struct LogEntry {
    /// Monotonic sequence number (unique across entire session)
    pub seq: u64,
    /// ISO 8601 timestamp with microseconds
    pub ts: String,
    /// Session ID
    pub session_id: String,
    /// Run ID (increments on restart within session)
    pub run_id: u64,
    /// Component that emitted the log
    pub component: String,
    /// Structured event data
    pub event: Value,
}

impl StructuredLogger {
    /// Creates a new structured logger for the given session.
    ///
    /// Logs are written to `<logs_dir>/events.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The logs directory cannot be created
    /// - The log file cannot be opened
    pub fn new(session_id: &str, logs_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(logs_dir)?;
        let log_path = logs_dir.join("events.jsonl");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        Ok(Self {
            session_id: session_id.to_string(),
            run_id: AtomicU64::new(1),
            seq: AtomicU64::new(0),
            log_file: Mutex::new(file),
            log_path,
        })
    }

    /// Increments the run ID (called when workflow restarts within a session).
    pub fn increment_run_id(&self) {
        self.run_id.fetch_add(1, Ordering::SeqCst);
    }

    /// Returns the next sequence number.
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Logs a structured event.
    ///
    /// The event is serialized to JSON and written as a single line.
    /// This method is thread-safe.
    pub fn log(&self, component: &str, event: impl Serialize) {
        let entry = LogEntry {
            seq: self.next_seq(),
            ts: Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
            session_id: self.session_id.clone(),
            run_id: self.run_id.load(Ordering::SeqCst),
            component: component.to_string(),
            event: serde_json::to_value(event).unwrap_or(Value::Null),
        };

        if let Ok(mut file) = self.log_file.lock() {
            if let Ok(line) = serde_json::to_string(&entry) {
                let _ = writeln!(file, "{}", line);
                let _ = file.flush();
            }
        }
    }

    /// Logs a domain workflow command.
    pub fn log_workflow_command(&self, command: &WorkflowCommand) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "WorkflowCommand",
                "command": command
            }),
        );
    }

    /// Logs a domain workflow event.
    pub fn log_workflow_event(&self, event: &WorkflowEvent) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "WorkflowEvent",
                "event": event
            }),
        );
    }

    /// Logs a channel send operation.
    pub fn log_channel_send(&self, channel: &str, message: &str) {
        self.log(
            "Channel",
            serde_json::json!({
                "type": "Send",
                "channel": channel,
                "message": message
            }),
        );
    }

    /// Logs a channel receive operation.
    pub fn log_channel_recv(&self, channel: &str, message: &str) {
        self.log(
            "Channel",
            serde_json::json!({
                "type": "Recv",
                "channel": channel,
                "message": message
            }),
        );
    }

    /// Logs a user input event.
    pub fn log_user_input(&self, key: &str, context: &str) {
        self.log(
            "TUI",
            serde_json::json!({
                "type": "UserInput",
                "key": key,
                "context": context
            }),
        );
    }

    /// Logs a workflow spawn event.
    pub fn log_workflow_spawn(&self, old_running: bool) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "WorkflowSpawned",
                "previous_workflow_running": old_running
            }),
        );
    }

    /// Logs a workflow completion event.
    pub fn log_workflow_complete(&self, result: &str) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "WorkflowComplete",
                "result": result
            }),
        );
    }

    /// Logs a concurrent workflow prevention event.
    pub fn log_concurrent_workflow_prevented(&self, reason: &str) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "ConcurrentWorkflowPrevented",
                "reason": reason
            }),
        );
    }

    /// Logs a phase transition event.
    pub fn log_phase_transition(&self, from: &str, to: &str) {
        self.log(
            "Workflow",
            serde_json::json!({
                "type": "PhaseTransition",
                "from": from,
                "to": to
            }),
        );
    }

    /// Logs an agent invocation event.
    pub fn log_agent_invocation(&self, agent: &str, phase: &str) {
        self.log(
            "Agent",
            serde_json::json!({
                "type": "Invocation",
                "agent": agent,
                "phase": phase
            }),
        );
    }

    /// Logs an agent completion event.
    pub fn log_agent_complete(&self, agent: &str, success: bool) {
        self.log(
            "Agent",
            serde_json::json!({
                "type": "Complete",
                "agent": agent,
                "success": success
            }),
        );
    }

    /// Returns the path to the log file.
    pub fn path(&self) -> &PathBuf {
        &self.log_path
    }

    /// Returns the current session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[cfg(test)]
#[path = "tests/structured_logger_tests.rs"]
mod tests;
