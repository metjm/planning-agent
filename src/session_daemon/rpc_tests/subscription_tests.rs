//! Subscription and callback tests for the RPC daemon.

use super::{create_test_record, TestServer};
use crate::rpc::daemon_service::SubscriberCallback;
use crate::rpc::{LivenessState, SessionRecord};
use std::time::Duration;
use tarpc::server::{self, Channel};
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_subscription_receives_session_changed() {
    let server = TestServer::start().await;

    let temp_dir = tempfile::tempdir().unwrap();
    let port_path = temp_dir.path().join("sessiond.port");
    server.write_port_file(&port_path);

    std::env::set_var("PLANNING_AGENT_HOME", temp_dir.path());

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

    std::env::remove_var("PLANNING_AGENT_HOME");
}

#[tokio::test]
async fn test_subscription_callback_end_to_end() {
    let server = TestServer::start().await;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = TestSubscriber { tx: event_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("callback-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await;

    match received {
        Ok(Some(record)) => {
            assert_eq!(record.workflow_session_id, "callback-test");
            assert_eq!(record.pid, 1000);
        }
        Ok(None) => panic!("Channel closed without receiving event"),
        Err(_) => panic!("Timeout waiting for subscription callback"),
    }
}

#[tokio::test]
async fn test_subscription_receives_multiple_events() {
    let server = TestServer::start().await;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = TestSubscriber { tx: event_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("multi-event-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let mut updated = create_test_record("multi-event-test", 1000);
    updated.phase = "Reviewing".to_string();
    client
        .update(tarpc::context::current(), updated)
        .await
        .unwrap()
        .unwrap();

    client
        .force_stop(tarpc::context::current(), "multi-event-test".to_string())
        .await
        .unwrap()
        .unwrap();

    let mut events = Vec::new();
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_secs(1), event_rx.recv()).await {
            Ok(Some(record)) => events.push(record),
            _ => break,
        }
    }

    assert_eq!(events.len(), 3, "Should receive 3 events");
    assert_eq!(events[0].phase, "Planning");
    assert_eq!(events[1].phase, "Reviewing");
    assert_eq!(events[2].liveness, LivenessState::Stopped);
}

#[tokio::test]
async fn test_multiple_subscribers_receive_events() {
    let server = TestServer::start().await;

    let (tx1, mut rx1) = mpsc::unbounded_channel::<SessionRecord>();
    let (tx2, mut rx2) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport1 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler1 = TestSubscriber { tx: tx1 };
    let channel1 = server::BaseChannel::with_defaults(transport1);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel1
            .execute(handler1.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    let transport2 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler2 = TestSubscriber { tx: tx2 };
    let channel2 = server::BaseChannel::with_defaults(transport2);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel2
            .execute(handler2.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("broadcast-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let event1 = tokio::time::timeout(Duration::from_secs(2), rx1.recv()).await;
    let event2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv()).await;

    assert!(
        event1.is_ok() && event1.unwrap().is_some(),
        "Subscriber 1 should receive event"
    );
    assert!(
        event2.is_ok() && event2.unwrap().is_some(),
        "Subscriber 2 should receive event"
    );
}

#[tokio::test]
async fn test_daemon_restarting_callback_end_to_end() {
    let server = TestServer::start().await;

    let (restart_tx, mut restart_rx) = mpsc::unbounded_channel::<String>();

    #[derive(Clone)]
    struct RestartSubscriber {
        restart_tx: mpsc::UnboundedSender<String>,
    }

    impl SubscriberCallback for RestartSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, _record: SessionRecord) {}

        async fn daemon_restarting(self, _: tarpc::context::Context, new_sha: String) {
            let _ = self.restart_tx.send(new_sha);
        }

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let subscriber_addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&subscriber_addr, Bincode::default)
        .await
        .unwrap();

    let handler = RestartSubscriber { restart_tx };
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|response| async {
                tokio::spawn(response);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let _ = client.shutdown(tarpc::context::current()).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), restart_rx.recv()).await;

    match received {
        Ok(Some(sha)) => {
            assert!(
                !sha.is_empty(),
                "daemon_restarting should include build SHA"
            );
        }
        Ok(None) => panic!("Channel closed without receiving daemon_restarting"),
        Err(_) => panic!("Timeout waiting for daemon_restarting callback"),
    }
}

#[tokio::test]
async fn test_subscriber_partial_failure_cleanup() {
    let server = TestServer::start().await;

    let (alive_tx, mut alive_rx) = mpsc::unbounded_channel::<SessionRecord>();
    let (dead_tx, _dead_rx) = mpsc::unbounded_channel::<SessionRecord>();

    #[derive(Clone)]
    struct TestSubscriber {
        tx: mpsc::UnboundedSender<SessionRecord>,
    }

    impl SubscriberCallback for TestSubscriber {
        async fn session_changed(self, _: tarpc::context::Context, record: SessionRecord) {
            let _ = self.tx.send(record);
        }

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport1 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler1 = TestSubscriber { tx: alive_tx };
    let channel1 = server::BaseChannel::with_defaults(transport1);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel1
            .execute(handler1.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    let transport2 = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler2 = TestSubscriber { tx: dead_tx };
    let channel2 = server::BaseChannel::with_defaults(transport2);

    let dead_handle = tokio::spawn(async move {
        use futures::StreamExt;
        channel2
            .execute(handler2.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    dead_handle.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = server.create_client().await;
    client
        .authenticate(tarpc::context::current(), server.auth_token.clone())
        .await
        .unwrap()
        .unwrap();

    let record = create_test_record("partial-failure-test", 1000);
    client
        .register(tarpc::context::current(), record)
        .await
        .unwrap()
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), alive_rx.recv()).await;
    assert!(
        received.is_ok() && received.unwrap().is_some(),
        "Alive subscriber should still receive events after partial failure"
    );

    let record2 = create_test_record("partial-failure-test-2", 2000);
    client
        .register(tarpc::context::current(), record2)
        .await
        .unwrap()
        .unwrap();

    let received2 = tokio::time::timeout(Duration::from_secs(2), alive_rx.recv()).await;
    assert!(
        received2.is_ok() && received2.unwrap().is_some(),
        "Alive subscriber should continue receiving after dead subscriber cleanup"
    );
}

#[tokio::test]
async fn test_subscriber_ping_detects_healthy_subscriber() {
    use crate::session_daemon::rpc_server::SubscriberRegistry;

    let server = TestServer::start().await;

    #[derive(Clone)]
    struct HealthySubscriber;

    impl SubscriberCallback for HealthySubscriber {
        async fn session_changed(self, _: tarpc::context::Context, _record: SessionRecord) {}

        async fn daemon_restarting(self, _: tarpc::context::Context, _new_sha: String) {}

        async fn ping(self, _: tarpc::context::Context) -> bool {
            true
        }
    }

    let addr = format!("127.0.0.1:{}", server.subscriber_port);
    let transport = tarpc::serde_transport::tcp::connect(&addr, Bincode::default)
        .await
        .unwrap();
    let handler = HealthySubscriber;
    let channel = server::BaseChannel::with_defaults(transport);

    tokio::spawn(async move {
        use futures::StreamExt;
        channel
            .execute(handler.serve())
            .for_each(|r| async {
                tokio::spawn(r);
            })
            .await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let registry = SubscriberRegistry::new();

    assert_eq!(registry.count(), 0);
}
