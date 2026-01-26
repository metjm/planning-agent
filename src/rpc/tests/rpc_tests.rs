use super::*;

#[test]
fn test_daemon_error_display_session_not_found() {
    let err = DaemonError::SessionNotFound {
        session_id: "sess-123".to_string(),
    };
    assert_eq!(format!("{}", err), "Session not found: sess-123");
}

#[test]
fn test_daemon_error_display_already_registered() {
    let err = DaemonError::AlreadyRegistered {
        session_id: "sess-456".to_string(),
        existing_pid: 9999,
    };
    assert_eq!(
        format!("{}", err),
        "Session sess-456 already registered by PID 9999"
    );
}

#[test]
fn test_daemon_error_display_shutting_down() {
    let err = DaemonError::ShuttingDown;
    assert_eq!(format!("{}", err), "Daemon is shutting down");
}

#[test]
fn test_daemon_error_display_authentication_failed() {
    let err = DaemonError::AuthenticationFailed;
    assert_eq!(format!("{}", err), "Authentication failed");
}

#[test]
fn test_daemon_error_display_internal() {
    let err = DaemonError::Internal {
        message: "something went wrong".to_string(),
    };
    assert_eq!(format!("{}", err), "Internal error: something went wrong");
}

#[test]
fn test_host_error_display_protocol_mismatch() {
    let err = HostError::ProtocolMismatch {
        got: 1,
        expected: 2,
    };
    assert_eq!(
        format!("{}", err),
        "Protocol version mismatch: got 1, expected 2"
    );
}

#[test]
fn test_host_error_display_container_not_registered() {
    let err = HostError::ContainerNotRegistered;
    assert_eq!(format!("{}", err), "Container not registered");
}

#[test]
fn test_daemon_error_serialization() {
    let err = DaemonError::SessionNotFound {
        session_id: "test-session".to_string(),
    };
    let json = serde_json::to_string(&err).unwrap();
    let parsed: DaemonError = serde_json::from_str(&json).unwrap();
    match parsed {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "test-session");
        }
        _ => panic!("Expected SessionNotFound"),
    }
}

#[test]
fn test_host_error_serialization() {
    let err = HostError::ProtocolMismatch {
        got: 1,
        expected: 2,
    };
    let json = serde_json::to_string(&err).unwrap();
    let parsed: HostError = serde_json::from_str(&json).unwrap();
    match parsed {
        HostError::ProtocolMismatch { got, expected } => {
            assert_eq!(got, 1);
            assert_eq!(expected, 2);
        }
        _ => panic!("Expected ProtocolMismatch"),
    }
}
