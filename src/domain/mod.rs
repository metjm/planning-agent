//! Domain model for event-sourced workflow state management.
//!
//! This module provides a strongly typed CQRS/ES domain model that replaces
//! direct state mutations with command-driven state changes through an event log.
//!
//! # Architecture
//!
//! - **Commands** (`commands.rs`): Intent to change state
//! - **Events** (`events.rs`): Facts that have happened
//! - **Aggregate** (`aggregate.rs`): Command validation and event application
//! - **View** (`view.rs`): Read-only projection for UI and queries
//!
//! # Usage
//!
//! ```ignore
//! use crate::domain::{WorkflowCommand, WorkflowEvent, WorkflowAggregate};
//!
//! // Commands are dispatched through the actor or CQRS framework
//! let cmd = WorkflowCommand::CreateWorkflow { ... };
//!
//! // Events are applied to rebuild state
//! for event in events {
//!     view.apply_event(aggregate_id, &event, sequence);
//! }
//! ```

pub mod actor;
pub mod cqrs;
pub mod errors;
pub mod failure;
pub mod review;
pub mod services;
pub mod supervisor;
pub mod types;
pub mod view;

// Re-export CQRS types
pub use cqrs::*;

// Re-export commonly used types for convenience
pub use actor::{create_actor_args, WorkflowActor, WorkflowActorArgs, WorkflowMessage};
pub use errors::WorkflowError;
pub use failure::{FailureContext, FailureKind, FailurePolicy, RecoveryAction};
pub use review::{ReviewMode, SequentialReviewState, SerializableReviewResult};
pub use services::{WorkflowClock, WorkflowServices};
pub use supervisor::{SupervisorMsg, WorkflowSupervisor};
pub use types::{
    AgentConversationState, AgentId, FeatureName, FeedbackPath, FeedbackStatus,
    ImplementationPhase, ImplementationPhaseState, ImplementationVerdict, InvocationRecord,
    Iteration, MaxIterations, Objective, PhaseLabel, PlanPath, PlanningPhase, ResumeStrategy,
    TimestampUtc, UiMode, WorkflowId, WorkingDir, WorktreeState,
};
pub use view::{WorkflowEventEnvelope, WorkflowView};
