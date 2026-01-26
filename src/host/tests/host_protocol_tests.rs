use super::*;

#[test]
fn test_session_info_serialization() {
    let session = SessionInfo {
        session_id: "sess-123".to_string(),
        feature_name: "Test Feature".to_string(),
        phase: "Planning".to_string(),
        iteration: 1,
        status: "Running".to_string(),
        liveness: LivenessState::Running,
        started_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        pid: 0,
    };

    let json = serde_json::to_string(&session).unwrap();
    let parsed: SessionInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.session_id, "sess-123");
    assert_eq!(parsed.phase, "Planning");
}

#[test]
fn test_session_info_from_session_record() {
    use crate::session_daemon::protocol::SessionRecord;
    use std::path::PathBuf;

    let record = SessionRecord::new(
        "session-456".to_string(),
        "my-feature".to_string(),
        PathBuf::from("/work/dir"),
        PathBuf::from("/work/sessions/session-456"),
        "Reviewing".to_string(),
        2,
        "Under Review".to_string(),
        9999,
    );

    let session_info = SessionInfo::from_session_record(&record);

    assert_eq!(session_info.session_id, "session-456");
    assert_eq!(session_info.feature_name, "my-feature");
    assert_eq!(session_info.phase, "Reviewing");
    assert_eq!(session_info.iteration, 2);
    assert_eq!(session_info.status, "Under Review");
    assert_eq!(session_info.liveness, LivenessState::Running);
    // Both started_at and updated_at are set from record.updated_at
    assert_eq!(session_info.started_at, record.updated_at);
    assert_eq!(session_info.updated_at, record.updated_at);
}
