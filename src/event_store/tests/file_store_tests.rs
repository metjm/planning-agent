use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, WorkingDir,
};
use crate::domain::WorkflowCommand;
use cqrs_es::CqrsFramework;
use tempfile::tempdir;

fn build_cqrs_for_test() -> (
    tempfile::TempDir,
    CqrsFramework<WorkflowAggregate, FileEventStore>,
) {
    let dir = tempdir().expect("temp dir");
    let store = FileEventStore {
        log_path: dir.path().join("events.jsonl"),
        snapshot_path: dir.path().join("snapshot.json"),
        snapshot_every: 50,
    };
    let services = crate::domain::WorkflowServices::default();
    let queries: Vec<Box<dyn cqrs_es::Query<WorkflowAggregate>>> = Vec::new();
    (dir, CqrsFramework::new(store, queries, services))
}

#[tokio::test]
async fn test_create_workflow() {
    let (_dir, cqrs) = build_cqrs_for_test();
    let cmd = WorkflowCommand::CreateWorkflow {
        feature_name: FeatureName::from("test-feature"),
        objective: Objective::from("test objective"),
        working_dir: WorkingDir::from(std::path::PathBuf::from("/tmp").as_path()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath::from(std::path::PathBuf::from("/tmp/plan.md")),
        feedback_path: FeedbackPath::from(std::path::PathBuf::from("/tmp/feedback.md")),
    };

    let result = cqrs.execute("session-1", cmd).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_load_aggregate() {
    let (dir, cqrs) = build_cqrs_for_test();
    let cmd = WorkflowCommand::CreateWorkflow {
        feature_name: FeatureName::from("test-feature"),
        objective: Objective::from("test objective"),
        working_dir: WorkingDir::from(std::path::PathBuf::from("/tmp").as_path()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath::from(std::path::PathBuf::from("/tmp/plan.md")),
        feedback_path: FeedbackPath::from(std::path::PathBuf::from("/tmp/feedback.md")),
    };

    cqrs.execute("session-1", cmd).await.unwrap();

    // Create new store and load aggregate
    let store = FileEventStore {
        log_path: dir.path().join("events.jsonl"),
        snapshot_path: dir.path().join("snapshot.json"),
        snapshot_every: 50,
    };

    let ctx = store.load_aggregate("session-1").await.unwrap();
    assert_eq!(ctx.current_sequence, 1);
}

#[test]
fn test_should_snapshot() {
    assert!(!should_snapshot(49, 50));
    assert!(should_snapshot(50, 50));
    assert!(should_snapshot(100, 50));
    assert!(!should_snapshot(101, 50));
    assert!(!should_snapshot(50, 0)); // Disabled
}
