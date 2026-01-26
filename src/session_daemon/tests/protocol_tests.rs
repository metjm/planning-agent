use super::*;

#[test]
fn test_session_record_new() {
    let record = SessionRecord::new(
        "session-123".to_string(),
        "test-feature".to_string(),
        PathBuf::from("/test/dir"),
        PathBuf::from("/test/sessions/session-123"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        12345,
    );

    assert_eq!(record.workflow_session_id, "session-123");
    assert_eq!(record.liveness, LivenessState::Running);
    assert!(!record.updated_at.is_empty());
    assert!(!record.last_heartbeat_at.is_empty());
}

#[test]
fn test_session_record_update_heartbeat() {
    let mut record = SessionRecord::new(
        "session-123".to_string(),
        "test-feature".to_string(),
        PathBuf::from("/test/dir"),
        PathBuf::from("/test/sessions/session-123"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        12345,
    );

    record.liveness = LivenessState::Unresponsive;
    record.update_heartbeat();

    assert_eq!(record.liveness, LivenessState::Running);
}

#[test]
fn test_liveness_state_display() {
    assert_eq!(format!("{}", LivenessState::Running), "Running");
    assert_eq!(format!("{}", LivenessState::Unresponsive), "Unresponsive");
    assert_eq!(format!("{}", LivenessState::Stopped), "Stopped");
}

#[test]
fn test_port_file_content_serialization() {
    let content = PortFileContent {
        port: 12345,
        subscriber_port: 12346,
        token: "secret-token".to_string(),
    };

    let json = serde_json::to_string(&content).unwrap();
    let parsed: PortFileContent = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.port, 12345);
    assert_eq!(parsed.subscriber_port, 12346);
    assert_eq!(parsed.token, "secret-token");
}

#[test]
fn test_session_record_update_state() {
    let mut record = SessionRecord::new(
        "session-123".to_string(),
        "test-feature".to_string(),
        PathBuf::from("/test/dir"),
        PathBuf::from("/test/sessions/session-123"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        12345,
    );

    let old_updated_at = record.updated_at.clone();

    // Small delay to ensure timestamp changes
    std::thread::sleep(std::time::Duration::from_millis(10));

    record.update_state("Reviewing".to_string(), 2, "In Review".to_string());

    assert_eq!(record.phase, "Reviewing");
    assert_eq!(record.iteration, 2);
    assert_eq!(record.workflow_status, "In Review");
    assert_ne!(
        record.updated_at, old_updated_at,
        "updated_at should change"
    );
}

#[test]
fn test_session_record_serialization() {
    let record = SessionRecord::new(
        "session-123".to_string(),
        "test-feature".to_string(),
        PathBuf::from("/test/dir"),
        PathBuf::from("/test/sessions/session-123"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        12345,
    );

    let json = serde_json::to_string(&record).unwrap();
    let parsed: SessionRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.workflow_session_id, "session-123");
    assert_eq!(parsed.feature_name, "test-feature");
    assert_eq!(parsed.phase, "Planning");
    assert_eq!(parsed.iteration, 1);
    assert_eq!(parsed.pid, 12345);
    assert_eq!(parsed.liveness, LivenessState::Running);
}
