//! Integration tests for DaemonSubscription.
//!
//! These tests spawn a real daemon and verify push notification functionality.
//! Tests share a daemon instance - each test cleans up its own session
//! but does NOT shut down the daemon (which would break parallel tests).
//!
//! Each test uses unique session IDs (UUIDs) to avoid conflicts with:
//! - Other tests running in parallel
//! - Other planning-agent instances on the same system
//! - Leftover sessions from previous test runs

use super::subscription::DaemonSubscription;
use super::SessionDaemonClient;
use crate::session_daemon::protocol::{DaemonMessage, SessionRecord};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

/// Generate a unique session ID for this test run.
fn unique_session_id(prefix: &str) -> String {
    format!("{}-{}", prefix, Uuid::new_v4())
}

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

/// Wait for a notification for a specific session, filtering out notifications
/// from other sessions (which may come from parallel tests or other planning-agents).
async fn wait_for_session_notification(
    subscription: &mut DaemonSubscription,
    expected_session_id: &str,
    timeout: Duration,
) -> Result<SessionRecord, &'static str> {
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, subscription.recv()).await {
            Ok(Some(DaemonMessage::SessionChanged(record))) => {
                if record.workflow_session_id == expected_session_id {
                    return Ok(record);
                }
                // Not our session, keep waiting (notification from parallel test or other agent)
            }
            Ok(Some(_other)) => {
                // Other message type, keep waiting
            }
            Ok(None) => {
                return Err("Connection closed unexpectedly");
            }
            Err(_) => {
                return Err("Timeout waiting for notification");
            }
        }
    }
    Err("Timeout waiting for notification")
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
    assert!(
        subscription.is_some(),
        "Should be able to connect and subscribe"
    );
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

    let session_id = unique_session_id("sub-register");

    // Register a session - this should trigger a push notification
    let record = create_test_record(&session_id);
    client.register(record).await.expect("Register failed");

    // Wait for our notification (filtering out notifications from other sessions)
    match wait_for_session_notification(&mut subscription, &session_id, Duration::from_secs(2))
        .await
    {
        Ok(record) => {
            assert_eq!(record.workflow_session_id, session_id);
            assert_eq!(record.phase, "Planning");
        }
        Err(e) => {
            panic!("{}", e);
        }
    }

    cleanup_session(&client, &session_id).await;
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

    let session_id = unique_session_id("sub-update");

    // Register first
    let mut record = create_test_record(&session_id);
    client
        .register(record.clone())
        .await
        .expect("Register failed");

    // Consume register notification (filtered to our session)
    let _ =
        wait_for_session_notification(&mut subscription, &session_id, Duration::from_secs(1)).await;

    // Update the session
    record.phase = "Reviewing".to_string();
    record.iteration = 2;
    client.update(record).await.expect("Update failed");

    // Wait for update notification (filtered to our session)
    match wait_for_session_notification(&mut subscription, &session_id, Duration::from_secs(2))
        .await
    {
        Ok(record) => {
            assert_eq!(record.workflow_session_id, session_id);
            assert_eq!(record.phase, "Reviewing");
            assert_eq!(record.iteration, 2);
        }
        Err(e) => {
            panic!("{}", e);
        }
    }

    cleanup_session(&client, &session_id).await;
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

    let session_id = unique_session_id("sub-heartbeat");

    // Register first
    let record = create_test_record(&session_id);
    client.register(record).await.expect("Register failed");

    // Consume register notification (filtered to our session)
    let _ =
        wait_for_session_notification(&mut subscription, &session_id, Duration::from_secs(1)).await;

    // Send heartbeat
    client
        .heartbeat(&session_id)
        .await
        .expect("Heartbeat failed");

    // Wait for heartbeat notification (filtered to our session)
    match wait_for_session_notification(&mut subscription, &session_id, Duration::from_secs(2))
        .await
    {
        Ok(record) => {
            assert_eq!(record.workflow_session_id, session_id);
        }
        Err(e) => {
            panic!("{}", e);
        }
    }

    cleanup_session(&client, &session_id).await;
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

    let session_id = unique_session_id("sub-try-recv");

    // Drain any pending notifications from other tests/agents first
    while subscription.try_recv().is_some() {}

    // Register to generate a notification
    let record = create_test_record(&session_id);
    client.register(record).await.expect("Register failed");

    // Give time for notification to arrive
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Now try_recv should return some message (may need to filter through others)
    let mut found_our_session = false;
    for _ in 0..10 {
        match subscription.try_recv() {
            Some(DaemonMessage::SessionChanged(record)) => {
                if record.workflow_session_id == session_id {
                    found_our_session = true;
                    break;
                }
                // Not our session, continue looking
            }
            Some(_) => {
                // Other message type, continue
            }
            None => {
                // No more messages, wait a bit and try again
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    assert!(
        found_our_session,
        "Should have received notification for our session"
    );

    cleanup_session(&client, &session_id).await;
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

    let session_id_1 = unique_session_id("sub-multi-1");
    let session_id_2 = unique_session_id("sub-multi-2");

    // Register first session
    let record1 = create_test_record(&session_id_1);
    client.register(record1).await.expect("Register 1 failed");

    // Register second session
    let record2 = create_test_record(&session_id_2);
    client.register(record2).await.expect("Register 2 failed");

    // Wait for notifications for both of our sessions (filtering out others)
    let mut received_ids = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    while received_ids.len() < 2 && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, subscription.recv()).await {
            Ok(Some(DaemonMessage::SessionChanged(record))) => {
                // Only count notifications for our sessions
                if record.workflow_session_id == session_id_1
                    || record.workflow_session_id == session_id_2
                {
                    received_ids.insert(record.workflow_session_id.clone());
                }
            }
            Ok(Some(_)) => {
                // Other message type, continue
            }
            Ok(None) => {
                panic!("Connection closed unexpectedly");
            }
            Err(_) => {
                break; // Timeout
            }
        }
    }

    assert!(
        received_ids.contains(&session_id_1),
        "Should have received notification for session 1"
    );
    assert!(
        received_ids.contains(&session_id_2),
        "Should have received notification for session 2"
    );

    cleanup_session(&client, &session_id_1).await;
    cleanup_session(&client, &session_id_2).await;
}
