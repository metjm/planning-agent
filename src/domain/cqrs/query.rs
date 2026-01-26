//! CQRS query handler for workflow event projection.
//!
//! The WorkflowQuery applies events to the WorkflowView projection
//! and broadcasts them to subscribers via tokio channels.

use super::WorkflowAggregate;
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
#[path = "../tests/query_tests.rs"]
mod tests;
