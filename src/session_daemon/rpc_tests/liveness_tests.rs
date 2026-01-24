//! Liveness timeout tests for the RPC daemon.
//!
//! These tests modify global environment variables and must run serially.

use super::{create_test_record, TestServer};
use crate::rpc::LivenessState;

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_running_to_unresponsive_timeout() {
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let mut record = create_test_record("timeout-test", 1000);
    // With default timeouts (3s unresponsive, 10s stopped), 5s old = Unresponsive
    let past = chrono::Utc::now() - chrono::Duration::seconds(5);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        sessions[0].liveness,
        LivenessState::Unresponsive,
        "Session should be Unresponsive with 5s old timestamp (> 3s unresponsive threshold)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_unresponsive_to_stopped_timeout() {
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let mut record = create_test_record("stale-test", 1000);
    // With default timeouts (3s unresponsive, 10s stopped), 15s old = Stopped
    let past = chrono::Utc::now() - chrono::Duration::seconds(15);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Stopped,
        "Session should be Stopped with 15s old timestamp (> 10s stopped threshold)"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_liveness_heartbeat_resets_unresponsive() {
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let mut record = create_test_record("heartbeat-reset-test", 1000);
    // With default timeouts (3s unresponsive, 10s stopped), 5s old = Unresponsive
    let past = chrono::Utc::now() - chrono::Duration::seconds(5);
    record.last_heartbeat_at = past.to_rfc3339();
    record.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Unresponsive,
        "Session should be Unresponsive with 5s old timestamp"
    );

    client
        .heartbeat(
            tarpc::context::current(),
            "heartbeat-reset-test".to_string(),
        )
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Running,
        "Session should be Running after heartbeat"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_auto_stale_session_replacement() {
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let mut record1 = create_test_record("auto-replace-test", 1000);
    // With default timeouts (3s unresponsive, 10s stopped), 15s old = Stopped
    let past = chrono::Utc::now() - chrono::Duration::seconds(15);
    record1.last_heartbeat_at = past.to_rfc3339();
    record1.updated_at = past.to_rfc3339();

    client
        .register(tarpc::context::current(), record1)
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sessions[0].liveness,
        LivenessState::Stopped,
        "Original session should be Stopped"
    );

    let record2 = create_test_record("auto-replace-test", 2000);
    let result = client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Re-registration should succeed after auto-stale"
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
#[serial_test::serial]
async fn test_mixed_liveness_states_in_list() {
    std::env::remove_var("PLANNING_SESSIOND_UNRESPONSIVE_SECS");
    std::env::remove_var("PLANNING_SESSIOND_STALE_SECS");

    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let mut record_a = create_test_record("mixed-a", 1000);
    // With default timeouts (3s unresponsive, 10s stopped), 5s old = Unresponsive
    let past_a = chrono::Utc::now() - chrono::Duration::seconds(5);
    record_a.last_heartbeat_at = past_a.to_rfc3339();
    record_a.updated_at = past_a.to_rfc3339();

    client
        .register(tarpc::context::current(), record_a)
        .await
        .unwrap()
        .unwrap();

    let record_b = create_test_record("mixed-b", 2000);
    client
        .register(tarpc::context::current(), record_b)
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    let session_a = sessions
        .iter()
        .find(|s| s.workflow_session_id == "mixed-a")
        .expect("Session A should exist");
    let session_b = sessions
        .iter()
        .find(|s| s.workflow_session_id == "mixed-b")
        .expect("Session B should exist");

    assert_eq!(
        session_a.liveness,
        LivenessState::Unresponsive,
        "Session A should be Unresponsive (5s old)"
    );
    assert_eq!(
        session_b.liveness,
        LivenessState::Running,
        "Session B should be Running (current timestamp)"
    );
}
