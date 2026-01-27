//! Tests for workflow actor.

use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, Phase, PlanPath, WorkingDir,
};
use crate::planning_paths;
use tempfile::tempdir;

#[tokio::test]
async fn test_actor_handles_command() {
    let dir = tempdir().expect("temp dir");
    let _guard = planning_paths::set_home_for_test(dir.path().to_path_buf());
    let session_id = uuid::Uuid::new_v4().to_string();

    let (args, mut snapshot_rx, _event_rx) =
        create_actor_args(&session_id).expect("create args failed");

    let (actor_ref, _handle) = WorkflowActor::spawn(None, WorkflowActor, args)
        .await
        .expect("actor spawn failed");

    // Send CreateWorkflow command
    let (tx, rx) = oneshot::channel();
    let cmd = WorkflowCommand::CreateWorkflow {
        feature_name: FeatureName::from("test-feature"),
        objective: Objective::from("test objective"),
        working_dir: WorkingDir::from(std::path::PathBuf::from("/tmp").as_path()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath::from(std::path::PathBuf::from("/tmp/plan.md")),
        feedback_path: FeedbackPath::from(std::path::PathBuf::from("/tmp/feedback.md")),
    };

    actor_ref
        .send_message(WorkflowMessage::Command(Box::new(cmd), tx))
        .expect("send failed");

    let result = rx.await.expect("receive failed");
    assert!(result.is_ok());

    let view = result.unwrap();
    assert!(view.feature_name().is_some());
    assert_eq!(view.feature_name().unwrap().as_str(), "test-feature");

    // Wait for snapshot update
    snapshot_rx.changed().await.expect("snapshot changed");
    let snapshot = snapshot_rx.borrow();
    assert!(snapshot.feature_name().is_some());
}

#[tokio::test]
async fn test_actor_get_view() {
    let dir = tempdir().expect("temp dir");
    let _guard = planning_paths::set_home_for_test(dir.path().to_path_buf());
    let session_id = uuid::Uuid::new_v4().to_string();

    let (args, _, _) = create_actor_args(&session_id).expect("create args failed");

    let (actor_ref, _handle) = WorkflowActor::spawn(None, WorkflowActor, args)
        .await
        .expect("actor spawn failed");

    // Get initial view
    let (tx, rx) = oneshot::channel();
    actor_ref
        .send_message(WorkflowMessage::GetView(tx))
        .expect("send failed");

    let view = rx.await.expect("receive failed");
    assert!(view.feature_name().is_none()); // Not initialized yet
}

#[tokio::test]
async fn test_bootstrap_view_from_events() {
    let dir = tempdir().expect("temp dir");
    let _guard = planning_paths::set_home_for_test(dir.path().to_path_buf());
    let session_id = uuid::Uuid::new_v4().to_string();

    // First create a workflow and persist events
    let (args, _, _) = create_actor_args(&session_id).expect("create args failed");
    let log_path = args.log_path.clone();

    let (actor_ref, _handle) = WorkflowActor::spawn(None, WorkflowActor, args)
        .await
        .expect("actor spawn failed");

    // Send CreateWorkflow command
    let (tx, rx) = oneshot::channel();
    let cmd = WorkflowCommand::CreateWorkflow {
        feature_name: FeatureName::from("bootstrap-test"),
        objective: Objective::from("test bootstrap"),
        working_dir: WorkingDir::from(std::path::PathBuf::from("/tmp").as_path()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath::from(std::path::PathBuf::from("/tmp/plan.md")),
        feedback_path: FeedbackPath::from(std::path::PathBuf::from("/tmp/feedback.md")),
    };
    actor_ref
        .send_message(WorkflowMessage::Command(Box::new(cmd), tx))
        .expect("send failed");
    let _ = rx.await.expect("receive failed");

    // Stop the actor
    actor_ref.stop(None);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Now bootstrap a fresh view from the event log
    let bootstrapped_view = bootstrap_view_from_events(&log_path, &session_id);

    // Verify the view was populated from persisted events
    assert!(bootstrapped_view.feature_name().is_some());
    assert_eq!(
        bootstrapped_view.feature_name().unwrap().as_str(),
        "bootstrap-test"
    );
    assert_eq!(bootstrapped_view.planning_phase(), Some(Phase::Planning));
    assert_eq!(bootstrapped_view.last_event_sequence(), 1);
}

#[test]
fn test_bootstrap_view_nonexistent_log() {
    let log_path = std::path::PathBuf::from("/nonexistent/path/events.jsonl");
    let view = bootstrap_view_from_events(&log_path, "any-id");

    // Should return default view without error
    assert!(view.feature_name().is_none());
    assert_eq!(view.last_event_sequence(), 0);
}
