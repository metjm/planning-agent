//! CQRS query handler for workflow event projection.
//!
//! The WorkflowQuery applies events to the WorkflowView projection
//! and broadcasts them to subscribers via tokio channels.

use crate::domain::aggregate::WorkflowAggregate;
use crate::domain::view::{WorkflowEventEnvelope, WorkflowView};
use async_trait::async_trait;
use cqrs_es::Query;
use std::sync::Arc;
use tokio::sync::{broadcast, watch, RwLock};

/// CQRS query handler that maintains the WorkflowView projection.
pub struct WorkflowQuery {
    /// In-memory projection of the workflow state.
    pub projection: Arc<RwLock<WorkflowView>>,
    /// Watch channel for snapshot updates (latest view).
    pub snapshot_tx: watch::Sender<WorkflowView>,
    /// Broadcast channel for event streaming.
    pub event_tx: broadcast::Sender<WorkflowEventEnvelope>,
}

impl WorkflowQuery {
    /// Creates a new workflow query handler.
    pub fn new(
        projection: Arc<RwLock<WorkflowView>>,
        snapshot_tx: watch::Sender<WorkflowView>,
        event_tx: broadcast::Sender<WorkflowEventEnvelope>,
    ) -> Self {
        Self {
            projection,
            snapshot_tx,
            event_tx,
        }
    }
}

#[async_trait]
impl Query<WorkflowAggregate> for WorkflowQuery {
    async fn dispatch(
        &self,
        aggregate_id: &str,
        events: &[cqrs_es::EventEnvelope<WorkflowAggregate>],
    ) {
        let mut view = self.projection.write().await;

        for event in events {
            // Apply event to projection
            view.apply_event(aggregate_id, &event.payload, event.sequence as u64);

            // Broadcast event to subscribers
            let envelope = WorkflowEventEnvelope::from(event);
            if let Err(e) = self.event_tx.send(envelope) {
                tracing::warn!("Failed to broadcast event: {:?}", e);
            }
        }

        // Send updated view snapshot
        let _ = self.snapshot_tx.send(view.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::events::WorkflowEvent;
    use crate::domain::types::{
        FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
    };
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
        assert!(updated_view.feature_name.is_some());
        assert_eq!(updated_view.feature_name.as_ref().unwrap().as_str(), "test");

        // Check snapshot was sent
        snapshot_rx.changed().await.unwrap();
        let snapshot = snapshot_rx.borrow();
        assert!(snapshot.feature_name.is_some());

        // Check event was broadcast
        let received = event_rx.try_recv().unwrap();
        assert_eq!(received.aggregate_id, aggregate_id);
        assert_eq!(received.sequence, 1);
    }
}
