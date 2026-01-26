//! Error types for the workflow domain.

use std::fmt::{Display, Formatter};

/// Errors that can occur during workflow command handling.
#[derive(Debug, Clone)]
pub enum WorkflowError {
    /// Invalid state transition attempted.
    InvalidTransition { message: String },
    /// Storage/persistence failure.
    StorageFailure { message: String },
    /// Command executed on uninitialized aggregate.
    NotInitialized,
    /// Optimistic lock failure (concurrent modification detected).
    ConcurrencyConflict { message: String },
}

impl Display for WorkflowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { message } => write!(f, "invalid transition: {}", message),
            Self::StorageFailure { message } => write!(f, "storage failure: {}", message),
            Self::NotInitialized => write!(f, "workflow not initialized"),
            Self::ConcurrencyConflict { message } => write!(f, "concurrency conflict: {}", message),
        }
    }
}

impl std::error::Error for WorkflowError {}
