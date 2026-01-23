//! Session lifecycle tests for the RPC daemon.

use super::{create_test_record, TestServer};
use crate::rpc::{DaemonError, LivenessState};

#[tokio::test]
async fn test_register_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("session-1", 1000);
    let result = client
        .register(tarpc::context::current(), record)
        .await
        .unwrap();

    assert!(result.is_ok(), "Register should succeed");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workflow_session_id, "session-1");
    assert_eq!(sessions[0].pid, 1000);
}

#[tokio::test]
async fn test_register_replaces_stale_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    client
        .force_stop(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap()
        .unwrap();

    let record2 = create_test_record("session-1", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Re-register should succeed for stopped session"
    );

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].pid, 2000);
    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_update_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let mut updated = create_test_record("session-1", 1000);
    updated.phase = "Reviewing".to_string();
    updated.iteration = 2;

    let result = client
        .update(tarpc::context::current(), updated)
        .await
        .unwrap();

    assert!(result.is_ok(), "Update should succeed");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].phase, "Reviewing");
    assert_eq!(sessions[0].iteration, 2);
}

#[tokio::test]
async fn test_heartbeat_updates_liveness() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let result = client
        .heartbeat(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    assert!(result.is_ok(), "Heartbeat should succeed");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_heartbeat_unknown_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let result = client
        .heartbeat(tarpc::context::current(), "unknown-session".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::SessionNotFound { .. })),
        "Heartbeat for unknown session should fail"
    );
}

#[tokio::test]
async fn test_force_stop_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let result = client
        .force_stop(tarpc::context::current(), "session-1".to_string())
        .await
        .unwrap();

    assert!(result.is_ok(), "Force stop should succeed");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions[0].liveness, LivenessState::Stopped);
}

#[tokio::test]
async fn test_force_stop_unknown_session() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let result = client
        .force_stop(tarpc::context::current(), "unknown-session".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::SessionNotFound { .. })),
        "Force stop for unknown session should fail"
    );
}

#[tokio::test]
async fn test_list_multiple_sessions() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    for i in 1..=5 {
        let record = create_test_record(&format!("session-{}", i), 1000 + i);
        client
            .register(tarpc::context::current(), record)
            .await
            .unwrap()
            .unwrap();
    }

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions.len(), 5);
}

#[tokio::test]
async fn test_shutdown_request() {
    use std::time::Duration;

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let result = client.shutdown(tarpc::context::current()).await.unwrap();

    assert!(result.is_ok(), "Shutdown should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;

    assert!(connect_result.is_err(), "Server should be shut down");
}

#[tokio::test]
async fn test_error_session_not_found() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let result = client
        .heartbeat(tarpc::context::current(), "nonexistent".to_string())
        .await
        .unwrap();

    match result {
        Err(DaemonError::SessionNotFound { session_id }) => {
            assert_eq!(session_id, "nonexistent");
        }
        _ => panic!("Expected SessionNotFound error"),
    }
}

#[tokio::test]
async fn test_error_authentication_failed() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let result = client
        .authenticate(tarpc::context::current(), "bad-token".to_string())
        .await
        .unwrap();

    assert!(matches!(result, Err(DaemonError::AuthenticationFailed)));
}

#[tokio::test]
async fn test_update_creates_session_if_missing() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("new-session", 1000);
    let result = client
        .update(tarpc::context::current(), record)
        .await
        .unwrap();

    assert!(result.is_ok(), "Update should create session if missing");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].workflow_session_id, "new-session");
}

#[tokio::test]
async fn test_error_already_registered() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    let record2 = create_test_record("session-1", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    match result {
        Err(DaemonError::AlreadyRegistered {
            session_id,
            existing_pid,
        }) => {
            assert_eq!(session_id, "session-1");
            assert_eq!(existing_pid, 1000);
        }
        _ => panic!("Expected AlreadyRegistered error, got {:?}", result),
    }
}

#[tokio::test]
async fn test_register_same_pid_succeeds() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record1 = create_test_record("session-1", 1000);
    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    let mut record2 = create_test_record("session-1", 1000);
    record2.phase = "Reviewing".to_string();
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(result.is_ok(), "Re-register with same PID should succeed");

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].phase, "Reviewing");
}

#[tokio::test]
#[serial_test::serial]
async fn test_state_persistence_saves_and_loads_sessions() {
    use crate::rpc::LivenessState;
    use crate::session_daemon::server::DaemonState;

    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

    let mut state = DaemonState::new();

    let record1 = create_test_record("persist-1", 1000);
    let record2 = create_test_record("persist-2", 2000);

    state
        .sessions
        .insert(record1.workflow_session_id.clone(), record1);
    state
        .sessions
        .insert(record2.workflow_session_id.clone(), record2);

    let persist_result = state.persist_to_disk();
    assert!(persist_result.is_ok(), "Persist should succeed");

    let mut loaded_state = DaemonState::new();
    let load_result = loaded_state.load_from_disk();
    assert!(load_result.is_ok(), "Load should succeed");

    assert_eq!(loaded_state.sessions.len(), 2, "Should have 2 sessions");
    assert!(loaded_state.sessions.contains_key("persist-1"));
    assert!(loaded_state.sessions.contains_key("persist-2"));

    assert_eq!(
        loaded_state.sessions.get("persist-1").unwrap().liveness,
        LivenessState::Stopped,
        "Loaded sessions should be marked Stopped"
    );
    assert_eq!(
        loaded_state.sessions.get("persist-2").unwrap().liveness,
        LivenessState::Stopped,
        "Loaded sessions should be marked Stopped"
    );

    std::env::remove_var("PLANNING_AGENT_HOME");
}

#[tokio::test]
#[serial_test::serial]
async fn test_state_persistence_handles_missing_file() {
    use crate::session_daemon::server::DaemonState;

    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

    let mut state = DaemonState::new();
    let result = state.load_from_disk();

    assert!(result.is_ok(), "Load should succeed even with no file");
    assert_eq!(state.sessions.len(), 0, "Should have no sessions");

    std::env::remove_var("PLANNING_AGENT_HOME");
}

#[tokio::test]
async fn test_degraded_mode_register_succeeds_silently() {
    use crate::session_daemon::rpc_client::RpcClient;

    let client = RpcClient::new(true).await;

    assert!(!client.is_connected(), "Client should be in degraded mode");

    let record = create_test_record("degraded-test", 1000);
    let result = client.register(record).await;

    assert!(result.is_ok(), "Register should succeed in degraded mode");
    assert_eq!(
        result.unwrap(),
        "",
        "Should return empty string in degraded mode"
    );
}

#[tokio::test]
async fn test_degraded_mode_all_operations_succeed() {
    use crate::session_daemon::rpc_client::RpcClient;

    let client = RpcClient::new(true).await;
    assert!(!client.is_connected());

    let record = create_test_record("degraded-test", 1000);

    let result = client.register(record.clone()).await;
    assert!(result.is_ok());

    let result = client.update(record).await;
    assert!(result.is_ok());

    let result = client.heartbeat("degraded-test").await;
    assert!(result.is_ok());

    let result = client.list().await;
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().len(),
        0,
        "List should return empty in degraded mode"
    );

    let result = client.force_stop("degraded-test").await;
    assert!(result.is_ok());

    let result = client.shutdown().await;
    assert!(result.is_ok());
}
