//! Workflow actor for CQRS command handling.
//!
//! The WorkflowActor wraps the CQRS framework and provides a message-based
//! interface for executing commands and querying state.

use crate::domain::cqrs::WorkflowAggregate;
use crate::domain::errors::WorkflowError;
use crate::domain::services::WorkflowServices;
use crate::domain::view::{WorkflowEventEnvelope, WorkflowView};
use crate::domain::WorkflowCommand;
use crate::domain::WorkflowQuery;
use crate::event_store::{FileEventStore, StoredEvent};
use crate::planning_paths;
use async_trait::async_trait;
use cqrs_es::{AggregateError, CqrsFramework};
use ractor::{Actor, ActorProcessingErr, ActorRef};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, watch, RwLock};

/// Messages that can be sent to the workflow actor.
pub enum WorkflowMessage {
    /// Execute a command and return the updated view (or error).
    Command(
        Box<WorkflowCommand>,
        oneshot::Sender<Result<WorkflowView, WorkflowError>>,
    ),
    /// Get the current view.
    GetView(oneshot::Sender<WorkflowView>),
}

/// Arguments for spawning a workflow actor.
#[derive(Clone)]
pub struct WorkflowActorArgs {
    /// The aggregate ID (workflow session ID).
    pub aggregate_id: String,
    /// Path to the event log file.
    pub log_path: PathBuf,
    /// Path to the snapshot file.
    pub snapshot_path: PathBuf,
    /// Snapshot after every N events.
    pub snapshot_every: u64,
    /// Shared view for projection.
    pub view: Arc<RwLock<WorkflowView>>,
    /// Watch channel sender for view snapshots.
    pub snapshot_tx: watch::Sender<WorkflowView>,
    /// Broadcast channel sender for event streaming.
    pub event_tx: broadcast::Sender<WorkflowEventEnvelope>,
    /// Services for command handling.
    pub services: WorkflowServices,
}

/// State maintained by the workflow actor.
pub struct WorkflowActorState {
    /// The CQRS framework instance.
    pub cqrs: CqrsFramework<WorkflowAggregate, FileEventStore>,
    /// The aggregate ID.
    pub aggregate_id: String,
    /// Shared view for reading.
    pub view: Arc<RwLock<WorkflowView>>,
}

/// The workflow actor.
pub struct WorkflowActor;

impl WorkflowActor {
    /// Builds the CQRS framework from actor arguments.
    pub fn build_cqrs(
        args: &WorkflowActorArgs,
    ) -> CqrsFramework<WorkflowAggregate, FileEventStore> {
        let store = FileEventStore::new(
            args.log_path.clone(),
            args.snapshot_path.clone(),
            args.snapshot_every,
        );

        let query = WorkflowQuery::new(
            args.view.clone(),
            args.snapshot_tx.clone(),
            args.event_tx.clone(),
        );

        CqrsFramework::new(store, vec![Box::new(query)], args.services.clone())
    }
}

#[async_trait]
impl Actor for WorkflowActor {
    type Msg = WorkflowMessage;
    type State = WorkflowActorState;
    type Arguments = WorkflowActorArgs;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let cqrs = WorkflowActor::build_cqrs(&args);

        Ok(WorkflowActorState {
            cqrs,
            aggregate_id: args.aggregate_id,
            view: args.view,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            WorkflowMessage::Command(boxed_cmd, reply) => {
                let cmd = *boxed_cmd;
                let result = state.cqrs.execute(&state.aggregate_id, cmd).await;
                let view = state.view.read().await.clone();

                let mapped = match result {
                    Ok(()) => Ok(view),
                    Err(AggregateError::UserError(err)) => Err(err),
                    Err(AggregateError::AggregateConflict) => {
                        Err(WorkflowError::ConcurrencyConflict {
                            message: "aggregate was modified concurrently".to_string(),
                        })
                    }
                    Err(err) => Err(WorkflowError::StorageFailure {
                        message: err.to_string(),
                    }),
                };

                if reply.send(mapped).is_err() {
                    tracing::debug!("Command reply channel closed");
                }
            }
            WorkflowMessage::GetView(reply) => {
                let view = state.view.read().await.clone();
                if reply.send(view).is_err() {
                    tracing::debug!("Command reply channel closed");
                }
            }
        }

        Ok(())
    }
}

/// Bootstraps a WorkflowView by replaying events from an event log file.
///
/// This function reads all events for the given aggregate_id from the event log
/// and applies them to a fresh WorkflowView. This is used when resuming workflows
/// to restore the view state from persisted events.
///
/// Returns `WorkflowView::default()` if the log file doesn't exist.
pub fn bootstrap_view_from_events(log_path: &PathBuf, aggregate_id: &str) -> WorkflowView {
    let mut view = WorkflowView::default();

    let file = match File::open(log_path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return view,
        Err(_) => return view, // Return default on any error
    };

    let reader = BufReader::new(file);
    let mut skipped_lines = 0;

    for line in reader.lines().map_while(Result::ok) {
        if let Ok(stored) = serde_json::from_str::<StoredEvent>(&line) {
            if stored.aggregate_id == aggregate_id {
                view.apply_event(&stored.aggregate_id, &stored.event, stored.sequence);
            }
        } else {
            skipped_lines += 1;
        }
    }

    if skipped_lines > 0 {
        tracing::warn!("Skipped {} unparseable lines in event log", skipped_lines);
    }

    view
}

/// Helper to create actor arguments with default configuration.
///
/// Takes a session_id (workflow session ID) and uses the planning_paths helpers
/// to compute the event log and snapshot paths.
///
/// For resumed workflows, this function bootstraps the initial WorkflowView
/// by replaying events from the event log. For new workflows, the view starts
/// empty and will be populated when the first CreateWorkflow command is sent.
pub fn create_actor_args(
    session_id: &str,
) -> anyhow::Result<(
    WorkflowActorArgs,
    watch::Receiver<WorkflowView>,
    broadcast::Receiver<WorkflowEventEnvelope>,
)> {
    let log_path = planning_paths::session_event_log_path(session_id)?;
    let snapshot_path = planning_paths::session_aggregate_snapshot_path(session_id)?;

    // Bootstrap the view from existing events (if any)
    let initial_view = bootstrap_view_from_events(&log_path, session_id);
    let view = Arc::new(RwLock::new(initial_view.clone()));
    let (snapshot_tx, snapshot_rx) = watch::channel(initial_view);
    let (event_tx, event_rx) = broadcast::channel(64);

    let args = WorkflowActorArgs {
        aggregate_id: session_id.to_string(),
        log_path,
        snapshot_path,
        snapshot_every: 50,
        view,
        snapshot_tx,
        event_tx,
        services: WorkflowServices::default(),
    };

    Ok((args, snapshot_rx, event_rx))
}

#[cfg(test)]
#[path = "tests/actor_tests.rs"]
mod tests;
