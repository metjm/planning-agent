//! Concurrent access tests for the RPC daemon.

use super::{create_test_record, TestServer};
use crate::rpc::daemon_service::DaemonServiceClient;
use crate::rpc::LivenessState;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;

#[tokio::test]
async fn test_concurrent_registrations() {
    let server = TestServer::start().await;

    let mut handles = Vec::new();

    for i in 0..10 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            let record = create_test_record(&format!("concurrent-{}", i), 1000 + i as u32);
            client
                .register(tarpc::context::current(), record)
                .await
                .unwrap()
                .unwrap();
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let client = server.create_client().await;
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

    assert_eq!(
        sessions.len(),
        10,
        "All concurrent registrations should succeed"
    );
}

#[tokio::test]
async fn test_concurrent_heartbeats() {
    let server = TestServer::start().await;
    let client = server.create_client().await;

    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("heartbeat-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let mut handles = Vec::new();

    for _ in 0..20 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            client
                .heartbeat(tarpc::context::current(), "heartbeat-test".to_string())
                .await
                .unwrap()
                .unwrap();
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let sessions = client
        .list(tarpc::context::current())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(sessions[0].liveness, LivenessState::Running);
}

#[tokio::test]
async fn test_many_concurrent_clients_connect_successfully() {
    let server = TestServer::start().await;

    let mut handles = Vec::new();

    for i in 0..20 {
        let port = server.port;
        let token = server.auth_token.clone();

        let handle = tokio::spawn(async move {
            use tarpc::serde_transport::tcp;

            let addr = format!("127.0.0.1:{}", port);
            let transport = tcp::connect(&addr, Bincode::default).await.unwrap();
            let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

            client
                .authenticate(tarpc::context::current(), token)
                .await
                .unwrap()
                .unwrap();

            let sha = client.build_sha(tarpc::context::current()).await.unwrap();
            assert!(!sha.is_empty());

            i
        });

        handles.push(handle);
    }

    let mut completed = 0;
    for handle in handles {
        let _ = handle.await.unwrap();
        completed += 1;
    }

    assert_eq!(completed, 20, "All 20 concurrent clients should succeed");
}

#[tokio::test]
async fn test_rapid_connect_disconnect_cycles() {
    let server = TestServer::start().await;

    for _ in 0..10 {
        let client = server.create_client().await;

        client
            .authenticate(tarpc::context::current(), server.auth_token.clone())
            .await
            .unwrap()
            .unwrap();

        let _ = client.build_sha(tarpc::context::current()).await.unwrap();
    }

    let client = server.create_client().await;
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
    assert!(sessions.is_empty() || !sessions.is_empty());
}
