use super::*;
use tempfile::TempDir;

fn create_test_logger() -> (StructuredLogger, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let logger =
        StructuredLogger::new("test-session", temp_dir.path()).expect("Failed to create logger");
    (logger, temp_dir)
}

#[test]
fn test_log_entries_are_valid_json() {
    let (logger, temp_dir) = create_test_logger();

    // Log several entries
    logger.log("TestComponent", serde_json::json!({"key": "value1"}));
    logger.log("TestComponent", serde_json::json!({"key": "value2"}));
    logger.log("TestComponent", serde_json::json!({"key": "value3"}));

    // Read back and verify each line is valid JSON
    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    for line in content.lines() {
        let entry: LogEntry = serde_json::from_str(line).expect("Failed to parse log entry");
        assert_eq!(entry.session_id, "test-session");
        assert_eq!(entry.component, "TestComponent");
    }
}

#[test]
fn test_sequence_numbers_monotonic() {
    let (logger, temp_dir) = create_test_logger();

    // Log multiple entries
    for i in 0..10 {
        logger.log("Test", serde_json::json!({"iteration": i}));
    }

    // Verify sequence numbers are monotonically increasing
    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let mut prev_seq = 0u64;
    for line in content.lines() {
        let entry: LogEntry = serde_json::from_str(line).expect("Failed to parse log entry");
        assert!(
            entry.seq > prev_seq,
            "Sequence numbers should be monotonically increasing"
        );
        prev_seq = entry.seq;
    }
}

#[test]
fn test_run_id_increments() {
    let (logger, temp_dir) = create_test_logger();

    // Log with run_id 1
    logger.log("Test", serde_json::json!({"msg": "first"}));

    // Increment run_id
    logger.increment_run_id();

    // Log with run_id 2
    logger.log("Test", serde_json::json!({"msg": "second"}));

    // Verify run_ids
    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    let entry1: LogEntry = serde_json::from_str(lines[0]).expect("Failed to parse entry 1");
    let entry2: LogEntry = serde_json::from_str(lines[1]).expect("Failed to parse entry 2");

    assert_eq!(entry1.run_id, 1);
    assert_eq!(entry2.run_id, 2);
}

#[test]
fn test_concurrent_logging() {
    use std::sync::Arc;
    use std::thread;

    let (logger, temp_dir) = create_test_logger();
    let logger = Arc::new(logger);

    let mut handles = vec![];

    // Spawn multiple threads logging concurrently
    for t in 0..5 {
        let logger_clone = Arc::clone(&logger);
        let handle = thread::spawn(move || {
            for i in 0..20 {
                logger_clone.log("Thread", serde_json::json!({"thread": t, "iteration": i}));
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify all entries are valid JSON and count is correct
    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let mut count = 0;
    for line in content.lines() {
        let _entry: LogEntry = serde_json::from_str(line).expect("Failed to parse log entry");
        count += 1;
    }

    assert_eq!(count, 100); // 5 threads * 20 iterations
}

#[test]
fn test_timestamp_format() {
    let (logger, temp_dir) = create_test_logger();

    logger.log("Test", serde_json::json!({"msg": "test"}));

    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let entry: LogEntry =
        serde_json::from_str(content.lines().next().unwrap()).expect("Failed to parse entry");

    // Verify timestamp format: YYYY-MM-DDTHH:MM:SS.ffffffZ
    assert!(entry.ts.contains('T'));
    assert!(entry.ts.ends_with('Z'));
    assert!(entry.ts.contains('.'));
    // Microseconds should be 6 digits
    let micros_part = entry.ts.split('.').nth(1).unwrap();
    assert!(micros_part.len() >= 7); // 6 digits + 'Z'
}

#[test]
fn test_channel_logging() {
    let (logger, temp_dir) = create_test_logger();

    logger.log_channel_send("approval_tx", "Approve");
    logger.log_channel_recv("approval_rx", "Approve");

    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    let entry1: LogEntry = serde_json::from_str(lines[0]).expect("Failed to parse entry 1");
    assert_eq!(entry1.component, "Channel");
    assert_eq!(entry1.event["type"], "Send");
    assert_eq!(entry1.event["channel"], "approval_tx");

    let entry2: LogEntry = serde_json::from_str(lines[1]).expect("Failed to parse entry 2");
    assert_eq!(entry2.component, "Channel");
    assert_eq!(entry2.event["type"], "Recv");
    assert_eq!(entry2.event["channel"], "approval_rx");
}

#[test]
fn test_workflow_logging() {
    let (logger, temp_dir) = create_test_logger();

    logger.log_workflow_spawn(false);
    logger.log_workflow_complete("success");
    logger.log_concurrent_workflow_prevented("Previous workflow still running");

    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);

    let entry1: LogEntry = serde_json::from_str(lines[0]).expect("Failed to parse entry 1");
    assert_eq!(entry1.event["type"], "WorkflowSpawned");
    assert_eq!(entry1.event["previous_workflow_running"], false);

    let entry2: LogEntry = serde_json::from_str(lines[1]).expect("Failed to parse entry 2");
    assert_eq!(entry2.event["type"], "WorkflowComplete");
    assert_eq!(entry2.event["result"], "success");

    let entry3: LogEntry = serde_json::from_str(lines[2]).expect("Failed to parse entry 3");
    assert_eq!(entry3.event["type"], "ConcurrentWorkflowPrevented");
}

#[test]
fn test_domain_workflow_logging() {
    use crate::domain::types::{
        FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
    };
    use crate::domain::WorkflowCommand;
    use crate::domain::WorkflowEvent;

    let (logger, temp_dir) = create_test_logger();

    let command = WorkflowCommand::CreateWorkflow {
        feature_name: FeatureName("test-feature".to_string()),
        objective: Objective("Test objective".to_string()),
        working_dir: WorkingDir("/test/dir".into()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath("/test/plan.md".into()),
        feedback_path: FeedbackPath("/test/feedback.md".into()),
    };
    logger.log_workflow_command(&command);

    let event = WorkflowEvent::WorkflowCreated {
        feature_name: FeatureName("test-feature".to_string()),
        objective: Objective("Test objective".to_string()),
        working_dir: WorkingDir("/test/dir".into()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath("/test/plan.md".into()),
        feedback_path: FeedbackPath("/test/feedback.md".into()),
        created_at: TimestampUtc::now(),
    };
    logger.log_workflow_event(&event);

    let content = std::fs::read_to_string(temp_dir.path().join("events.jsonl"))
        .expect("Failed to read log file");

    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    let entry1: LogEntry = serde_json::from_str(lines[0]).expect("Failed to parse entry 1");
    assert_eq!(entry1.component, "Workflow");
    assert_eq!(entry1.event["type"], "WorkflowCommand");
    assert!(entry1.event["command"]["create_workflow"].is_object());

    let entry2: LogEntry = serde_json::from_str(lines[1]).expect("Failed to parse entry 2");
    assert_eq!(entry2.component, "Workflow");
    assert_eq!(entry2.event["type"], "WorkflowEvent");
    assert!(entry2.event["event"]["workflow_created"].is_object());
}
