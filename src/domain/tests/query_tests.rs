//! Tests for workflow query.

use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::WorkflowEvent;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[tokio::test]
async fn test_query_applies_event_to_view() {
    let view = Arc::new(RwLock::new(WorkflowView::default()));
    let (snapshot_tx, mut snapshot_rx) = watch::channel(WorkflowView::default());
    let (event_tx, mut event_rx) = broadcast::channel(16);

    let query = WorkflowQuery::new(view.clone(), snapshot_tx, event_tx);
    let aggregate_id = Uuid::new_v4().to_string();

    let event = WorkflowEvent::WorkflowCreated {
        feature_name: FeatureName::from("test"),
        objective: Objective::from("test objective"),
        working_dir: WorkingDir::from(PathBuf::from("/tmp").as_path()),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath::from(PathBuf::from("/tmp/plan.md")),
        feedback_path: FeedbackPath::from(PathBuf::from("/tmp/feedback.md")),
        created_at: TimestampUtc::now(),
    };

    let envelope = cqrs_es::EventEnvelope {
        aggregate_id: aggregate_id.clone(),
        sequence: 1,
        payload: event,
        metadata: HashMap::new(),
    };

    query.dispatch(&aggregate_id, &[envelope]).await;

    // Check view was updated
    let updated_view = view.read().await;
    assert!(updated_view.feature_name().is_some());
    assert_eq!(updated_view.feature_name().unwrap().as_str(), "test");

    // Check snapshot was sent
    snapshot_rx.changed().await.unwrap();
    let snapshot = snapshot_rx.borrow();
    assert!(snapshot.feature_name().is_some());

    // Check event was broadcast
    let received = event_rx.try_recv().unwrap();
    assert_eq!(received.aggregate_id, aggregate_id);
    assert_eq!(received.sequence, 1);
}
