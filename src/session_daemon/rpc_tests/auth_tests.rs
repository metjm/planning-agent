//! Authentication tests for the RPC daemon.

use super::TestServer;
use crate::rpc::DaemonError;

#[tokio::test]
async fn test_authentication_with_valid_token() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let result = client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap();

    assert!(
        result.is_ok(),
        "Authentication with valid token should succeed"
    );
}

#[tokio::test]
async fn test_authentication_with_invalid_token() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let result = client
        .authenticate(tarpc::context::current(), "wrong-token".to_string())
        .await
        .unwrap();

    assert!(
        matches!(result, Err(DaemonError::AuthenticationFailed)),
        "Authentication with invalid token should fail"
    );
}

#[tokio::test]
async fn test_unauthenticated_call_rejected() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    let result = client.list(tarpc::context::current()).await.unwrap();

    assert!(
        matches!(result, Err(DaemonError::AuthenticationFailed)),
        "Unauthenticated call should be rejected"
    );
}

#[tokio::test]
async fn test_authenticated_call_succeeds() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let result = client.list(tarpc::context::current()).await.unwrap();

    assert!(result.is_ok(), "Authenticated call should succeed");
    assert_eq!(
        result.unwrap().len(),
        0,
        "Should have no sessions initially"
    );
}

#[tokio::test]
async fn test_build_sha_returns_value() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    // build_sha doesn't require authentication
    let sha = client.build_sha(tarpc::context::current()).await.unwrap();

    assert!(!sha.is_empty(), "Build SHA should not be empty");
}
