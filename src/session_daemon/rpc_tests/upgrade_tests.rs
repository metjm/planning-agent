//! Version mismatch and upgrade tests for the RPC daemon.

use super::TestServer;
use std::time::Duration;

#[tokio::test]
async fn test_build_sha_is_consistent_for_same_binary() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_sha = client.build_sha(tarpc::context::current()).await.unwrap();
    let client_sha = crate::update::BUILD_SHA;

    assert_eq!(
        daemon_sha, client_sha,
        "Daemon and client from same binary should have same SHA"
    );
}

#[tokio::test]
async fn test_build_timestamp_is_consistent_for_same_binary() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();
    let client_timestamp = crate::update::BUILD_TIMESTAMP;

    assert_eq!(
        daemon_timestamp, client_timestamp,
        "Daemon and client from same binary should have same timestamp"
    );
    assert!(daemon_timestamp > 0, "Timestamp should be non-zero");
}

#[tokio::test]
async fn test_request_upgrade_with_newer_timestamp_accepted() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    let future_timestamp = daemon_timestamp + 1000;
    let accepted = client
        .request_upgrade(tarpc::context::current(), future_timestamp)
        .await
        .unwrap();

    assert!(accepted, "Upgrade from newer client should be accepted");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_err(), "Daemon should be shut down");
}

#[tokio::test]
async fn test_request_upgrade_with_older_timestamp_refused() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    let past_timestamp = daemon_timestamp.saturating_sub(1000);
    let accepted = client
        .request_upgrade(tarpc::context::current(), past_timestamp)
        .await
        .unwrap();

    assert!(!accepted, "Upgrade from older client should be refused");

    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(
        connect_result.is_ok(),
        "Daemon should still be running after refusing upgrade"
    );
}

#[tokio::test]
async fn test_request_upgrade_with_same_timestamp_refused() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    let accepted = client
        .request_upgrade(tarpc::context::current(), daemon_timestamp)
        .await
        .unwrap();

    assert!(
        !accepted,
        "Upgrade with same timestamp should be refused (not strictly newer)"
    );

    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_ok(), "Daemon should still be running");
}

#[tokio::test]
async fn test_old_client_cannot_kill_new_daemon() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let daemon_timestamp = client
        .build_timestamp(tarpc::context::current())
        .await
        .unwrap();

    let old_client_timestamp = daemon_timestamp.saturating_sub(365 * 24 * 60 * 60);

    let accepted = client
        .request_upgrade(tarpc::context::current(), old_client_timestamp)
        .await
        .unwrap();

    assert!(
        !accepted,
        "Old client should NOT be able to kill newer daemon"
    );

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn test_version_mismatch_triggers_shutdown() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let sha = client.build_sha(tarpc::context::current()).await.unwrap();
    assert!(!sha.is_empty(), "Build SHA should not be empty");

    let result = client.shutdown(tarpc::context::current()).await.unwrap();
    assert!(result.is_ok(), "Shutdown should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = format!("127.0.0.1:{}", server.port);
    let connect_result = tokio::net::TcpStream::connect(&addr).await;
    assert!(connect_result.is_err(), "Daemon should be shut down");
}
