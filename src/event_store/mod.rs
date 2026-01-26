//! File-based event store for workflow event sourcing.
//!
//! This module provides a JSONL-based event store with snapshot support
//! for the CQRS/ES workflow aggregate.

pub mod file_store;

pub use file_store::{FileAggregateContext, FileEventStore, StoredEvent, StoredSnapshot};
