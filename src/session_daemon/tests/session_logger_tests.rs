use super::*;
use crate::planning_paths::{set_home_for_test, TestHomeGuard};
use tempfile::tempdir;

/// Helper to set up an isolated test home directory.
fn test_env() -> (tempfile::TempDir, TestHomeGuard) {
    let dir = tempdir().expect("Failed to create temp dir");
    let guard = set_home_for_test(dir.path().to_path_buf());
    (dir, guard)
}

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
    let (_temp_dir, _guard) = test_env();

    let session_id = format!("test-{}", uuid::Uuid::new_v4());
    let result = SessionLogger::new(&session_id);
    assert!(result.is_ok());
}

#[test]
fn test_session_logger_logging() {
    let (_temp_dir, _guard) = test_env();

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
    let (_temp_dir, _guard) = test_env();

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
    let (_temp_dir, _guard) = test_env();

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
