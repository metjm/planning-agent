//! File-based event store implementation.
//!
//! Stores events as JSONL (one JSON object per line) with support for:
//! - Optimistic concurrency via file locking
//! - Snapshots for faster aggregate loading
//! - Atomic writes via temp file + rename

use crate::domain::errors::WorkflowError;
use crate::domain::types::TimestampUtc;
use crate::domain::WorkflowAggregate;
use crate::domain::WorkflowEvent;
use async_trait::async_trait;
use chrono::Utc;
use cqrs_es::{
    Aggregate, AggregateContext, AggregateError, DomainEvent, EventEnvelope, EventStore,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// A stored event record in the event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    pub aggregate_id: String,
    pub sequence: u64,
    pub recorded_at: TimestampUtc,
    pub event_type: String,
    pub event_version: String,
    pub event: WorkflowEvent,
    pub metadata: HashMap<String, String>,
}

/// A stored snapshot for faster aggregate loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSnapshot {
    pub aggregate_id: String,
    pub sequence: u64,
    pub snapshot_at: TimestampUtc,
    pub state: WorkflowAggregate,
}

/// File-based event store configuration.
#[derive(Debug, Clone)]
pub struct FileEventStore {
    /// Path to the JSONL event log file.
    pub log_path: PathBuf,
    /// Path to the JSON snapshot file.
    pub snapshot_path: PathBuf,
    /// Snapshot after every N events (0 = disabled).
    pub snapshot_every: u64,
}

/// Aggregate context for file-based storage.
pub struct FileAggregateContext<A: Aggregate> {
    /// The aggregate ID.
    pub aggregate_id: String,
    /// The rehydrated aggregate.
    pub aggregate: A,
    /// The current sequence number (last applied event).
    pub current_sequence: u64,
}

impl<A: Aggregate> AggregateContext<A> for FileAggregateContext<A> {
    fn aggregate(&self) -> &A {
        &self.aggregate
    }
}

impl FileEventStore {
    /// Creates a new file event store.
    pub fn new(log_path: PathBuf, snapshot_path: PathBuf, snapshot_every: u64) -> Self {
        Self {
            log_path,
            snapshot_path,
            snapshot_every,
        }
    }
}

#[async_trait]
impl EventStore<WorkflowAggregate> for FileEventStore {
    type AC = FileAggregateContext<WorkflowAggregate>;

    async fn load_events(
        &self,
        aggregate_id: &str,
    ) -> Result<Vec<EventEnvelope<WorkflowAggregate>>, AggregateError<WorkflowError>> {
        let file = match File::open(&self.log_path) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(AggregateError::UnexpectedError(Box::new(e))),
        };

        file.lock_shared()
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

        let reader = BufReader::new(file);
        let mut envelopes = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
            let stored: StoredEvent = serde_json::from_str(&line)
                .map_err(|e| AggregateError::DeserializationError(Box::new(e)))?;

            if stored.aggregate_id == aggregate_id {
                // Validate event type and version match
                if stored.event_type != stored.event.event_type()
                    || stored.event_version != stored.event.event_version()
                {
                    return Err(AggregateError::UnexpectedError(Box::new(
                        std::io::Error::new(ErrorKind::InvalidData, "event version/type mismatch"),
                    )));
                }

                envelopes.push(EventEnvelope {
                    aggregate_id: stored.aggregate_id,
                    sequence: stored.sequence as usize,
                    payload: stored.event,
                    metadata: stored.metadata,
                });
            }
        }

        Ok(envelopes)
    }

    async fn load_aggregate(
        &self,
        aggregate_id: &str,
    ) -> Result<Self::AC, AggregateError<WorkflowError>> {
        let mut aggregate = WorkflowAggregate::default();
        let mut current_sequence = 0u64;

        // Try to load from snapshot first
        if let Some(snapshot) = load_snapshot(&self.snapshot_path)? {
            if snapshot.aggregate_id == aggregate_id {
                aggregate = snapshot.state;
                current_sequence = snapshot.sequence;
            }
        }

        // Apply events after snapshot
        let events = self.load_events(aggregate_id).await?;
        for event in events {
            let seq = event.sequence as u64;
            if seq > current_sequence {
                current_sequence = seq;
                aggregate.apply(event.payload);
            }
        }

        Ok(FileAggregateContext {
            aggregate_id: aggregate_id.to_string(),
            aggregate,
            current_sequence,
        })
    }

    async fn commit(
        &self,
        events: Vec<WorkflowEvent>,
        context: Self::AC,
        metadata: HashMap<String, String>,
    ) -> Result<Vec<EventEnvelope<WorkflowAggregate>>, AggregateError<WorkflowError>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }

        // Ensure parent directory exists
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

        // Acquire exclusive lock for writing
        file.lock_exclusive()
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

        let FileAggregateContext {
            aggregate_id,
            mut aggregate,
            current_sequence,
        } = context;

        // Check for concurrent writes (optimistic concurrency)
        let last_sequence = read_last_sequence(&file, &aggregate_id)?;
        if last_sequence != current_sequence {
            return Err(AggregateError::AggregateConflict);
        }

        let mut sequence = current_sequence;
        let mut envelopes: Vec<EventEnvelope<WorkflowAggregate>> = Vec::new();

        for event in events {
            sequence += 1;

            let record = StoredEvent {
                aggregate_id: aggregate_id.clone(),
                sequence,
                recorded_at: TimestampUtc(Utc::now()),
                event_type: event.event_type(),
                event_version: event.event_version(),
                event: event.clone(),
                metadata: metadata.clone(),
            };

            let line = serde_json::to_string(&record)
                .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

            writeln!(file, "{}", line).map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

            envelopes.push(EventEnvelope {
                aggregate_id: aggregate_id.clone(),
                sequence: sequence as usize,
                payload: event,
                metadata: metadata.clone(),
            });
        }

        // Ensure all data is persisted
        file.flush()
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
        file.sync_all()
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

        // Apply events to aggregate for potential snapshot
        for envelope in &envelopes {
            let event: WorkflowEvent = envelope.payload.clone();
            aggregate.apply(event);
        }

        // Take snapshot if threshold reached
        if should_snapshot(sequence, self.snapshot_every) {
            let snapshot = StoredSnapshot {
                aggregate_id,
                sequence,
                snapshot_at: TimestampUtc(Utc::now()),
                state: aggregate,
            };
            save_snapshot(&self.snapshot_path, &snapshot)?;
        }

        Ok(envelopes)
    }
}

/// Load a snapshot from disk.
fn load_snapshot(path: &PathBuf) -> Result<Option<StoredSnapshot>, AggregateError<WorkflowError>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AggregateError::UnexpectedError(Box::new(e))),
    };

    let snapshot: StoredSnapshot = serde_json::from_str(&content)
        .map_err(|e| AggregateError::DeserializationError(Box::new(e)))?;

    Ok(Some(snapshot))
}

/// Save a snapshot to disk atomically.
fn save_snapshot(
    path: &PathBuf,
    snapshot: &StoredSnapshot,
) -> Result<(), AggregateError<WorkflowError>> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
    }

    let content = serde_json::to_string(snapshot)
        .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

    // Write to temp file, then rename for atomicity
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, content).map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
    std::fs::rename(&tmp_path, path).map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

    Ok(())
}

/// Read the last sequence number for an aggregate from the log file.
fn read_last_sequence(
    file: &File,
    aggregate_id: &str,
) -> Result<u64, AggregateError<WorkflowError>> {
    let mut reader = BufReader::new(
        file.try_clone()
            .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?,
    );

    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;

    let mut last_sequence = 0u64;

    for line in reader.lines() {
        let line = line.map_err(|e| AggregateError::UnexpectedError(Box::new(e)))?;
        let stored: StoredEvent = serde_json::from_str(&line)
            .map_err(|e| AggregateError::DeserializationError(Box::new(e)))?;

        if stored.aggregate_id == aggregate_id {
            last_sequence = stored.sequence;
        }
    }

    Ok(last_sequence)
}

/// Determines if a snapshot should be taken based on sequence and threshold.
fn should_snapshot(sequence: u64, snapshot_every: u64) -> bool {
    if snapshot_every == 0 {
        return false;
    }
    sequence.is_multiple_of(snapshot_every)
}

#[cfg(test)]
#[path = "tests/file_store_tests.rs"]
mod tests;
