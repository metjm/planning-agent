//! Integration tests for DaemonSubscription.
//!
//! These tests spawn a real daemon and verify push notification functionality.
//! Tests share a daemon instance - each test cleans up its own session
//! but does NOT shut down the daemon (which would break parallel tests).

use super::subscription::DaemonSubscription;
use super::SessionDaemonClient;
use crate::session_daemon::protocol::{DaemonMessage, SessionRecord};
use std::path::PathBuf;
use std::time::Duration;

fn create_test_record(id: &str) -> SessionRecord {
    SessionRecord::new(
        id.to_string(),
        "subscription-test-feature".to_string(),
        PathBuf::from("/tmp/subscription-test"),
        PathBuf::from("/tmp/subscription-test/state.json"),
        "Planning".to_string(),
        1,
        "Planning".to_string(),
        std::process::id(),
    )
}

/// Helper to clean up a specific session (marks it as stopped).
async fn cleanup_session(client: &SessionDaemonClient, session_id: &str) {
    let _ = client.force_stop(session_id).await;
}

#[tokio::test]
async fn test_subscription_connect() {
    // Ensure daemon is running by creating a client first
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    // Now try to subscribe
    let subscription = DaemonSubscription::connect().await;
    assert!(subscription.is_some(), "Should be able to connect and subscribe");
}

#[tokio::test]
async fn test_subscription_receives_register_notification() {
    // Start daemon via client
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    // Subscribe
    let mut subscription = match DaemonSubscription::connect().await {
        Some(s) => s,
        None => {
            println!("Skipping test - subscription not available");
            return;
        }
    };

    let session_id = "subscription-test-register";

    // Register a session - this should trigger a push notification
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Wait for push notification with timeout
    let notification = tokio::time::timeout(Duration::from_secs(2), subscription.recv()).await;

    match notification {
        Ok(Some(DaemonMessage::SessionChanged(record))) => {
            assert_eq!(record.workflow_session_id, session_id);
            assert_eq!(record.phase, "Planning");
        }
        Ok(Some(other)) => {
            panic!("Unexpected message: {:?}", other);
        }
        Ok(None) => {
            panic!("Connection closed unexpectedly");
        }
        Err(_) => {
            panic!("Timeout waiting for push notification");
        }
    }

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_subscription_receives_update_notification() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let mut subscription = match DaemonSubscription::connect().await {
        Some(s) => s,
        None => {
            println!("Skipping test - subscription not available");
            return;
        }
    };

    let session_id = "subscription-test-update";

    // Register first
    let mut record = create_test_record(session_id);
    client.register(record.clone()).await.expect("Register failed");

    // Consume register notification
    let _ = tokio::time::timeout(Duration::from_secs(1), subscription.recv()).await;

    // Update the session
    record.phase = "Reviewing".to_string();
    record.iteration = 2;
    client.update(record).await.expect("Update failed");

    // Wait for update notification
    let notification = tokio::time::timeout(Duration::from_secs(2), subscription.recv()).await;

    match notification {
        Ok(Some(DaemonMessage::SessionChanged(record))) => {
            assert_eq!(record.workflow_session_id, session_id);
            assert_eq!(record.phase, "Reviewing");
            assert_eq!(record.iteration, 2);
        }
        Ok(Some(other)) => {
            panic!("Unexpected message: {:?}", other);
        }
        Ok(None) => {
            panic!("Connection closed unexpectedly");
        }
        Err(_) => {
            panic!("Timeout waiting for update notification");
        }
    }

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_subscription_receives_heartbeat_notification() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let mut subscription = match DaemonSubscription::connect().await {
        Some(s) => s,
        None => {
            println!("Skipping test - subscription not available");
            return;
        }
    };

    let session_id = "subscription-test-heartbeat";

    // Register first
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Consume register notification
    let _ = tokio::time::timeout(Duration::from_secs(1), subscription.recv()).await;

    // Send heartbeat
    client.heartbeat(session_id).await.expect("Heartbeat failed");

    // Wait for heartbeat notification (updates last_heartbeat)
    let notification = tokio::time::timeout(Duration::from_secs(2), subscription.recv()).await;

    match notification {
        Ok(Some(DaemonMessage::SessionChanged(record))) => {
            assert_eq!(record.workflow_session_id, session_id);
        }
        Ok(Some(other)) => {
            panic!("Unexpected message: {:?}", other);
        }
        Ok(None) => {
            panic!("Connection closed unexpectedly");
        }
        Err(_) => {
            panic!("Timeout waiting for heartbeat notification");
        }
    }

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_subscription_try_recv() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let mut subscription = match DaemonSubscription::connect().await {
        Some(s) => s,
        None => {
            println!("Skipping test - subscription not available");
            return;
        }
    };

    // try_recv should return None when no messages pending
    assert!(subscription.try_recv().is_none());

    let session_id = "subscription-test-try-recv";

    // Register to generate a notification
    let record = create_test_record(session_id);
    client.register(record).await.expect("Register failed");

    // Give time for notification to arrive
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now try_recv should return the message
    let msg = subscription.try_recv();
    assert!(msg.is_some(), "Should have received notification");

    cleanup_session(&client, session_id).await;
}

#[tokio::test]
async fn test_subscription_multiple_sessions() {
    let client = SessionDaemonClient::new(false);
    tokio::time::sleep(Duration::from_millis(500)).await;

    if !client.is_connected() {
        println!("Skipping test - daemon not available");
        return;
    }

    let mut subscription = match DaemonSubscription::connect().await {
        Some(s) => s,
        None => {
            println!("Skipping test - subscription not available");
            return;
        }
    };

    let session_id_1 = "subscription-test-multi-1";
    let session_id_2 = "subscription-test-multi-2";

    // Register first session
    let record1 = create_test_record(session_id_1);
    client.register(record1).await.expect("Register 1 failed");

    // Register second session
    let record2 = create_test_record(session_id_2);
    client.register(record2).await.expect("Register 2 failed");

    // Should receive notifications for both
    let mut received_ids = Vec::new();
    for _ in 0..2 {
        let notification = tokio::time::timeout(Duration::from_secs(2), subscription.recv()).await;
        if let Ok(Some(DaemonMessage::SessionChanged(record))) = notification {
            received_ids.push(record.workflow_session_id.clone());
        }
    }

    assert!(
        received_ids.contains(&session_id_1.to_string()),
        "Should have received notification for session 1"
    );
    assert!(
        received_ids.contains(&session_id_2.to_string()),
        "Should have received notification for session 2"
    );

    cleanup_session(&client, session_id_1).await;
    cleanup_session(&client, session_id_2).await;
}
