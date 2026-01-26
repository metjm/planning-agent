//! External services for the workflow aggregate.
//!
//! Services provide external dependencies (like time) to the aggregate
//! without coupling it to specific implementations.

use crate::domain::types::TimestampUtc;

/// Services injected into the workflow aggregate for command handling.
#[derive(Debug, Clone, Default)]
pub struct WorkflowServices {
    pub clock: WorkflowClock,
}

/// Clock service for timestamp generation.
#[derive(Debug, Clone, Default)]
pub struct WorkflowClock;

impl WorkflowClock {
    /// Returns the current UTC timestamp.
    pub fn now(&self) -> TimestampUtc {
        TimestampUtc::now()
    }
}
