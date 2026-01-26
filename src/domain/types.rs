//! Strongly typed domain primitives for the workflow aggregate.
//!
//! These newtypes provide type safety and semantic clarity for workflow identifiers,
//! paths, and iteration counters. They are used throughout the domain model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a workflow session.
/// Used as the aggregate_id in the event store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    /// Creates a new random workflow ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates a workflow ID from a string.
    pub fn from_string(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Human-readable feature name for the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureName(pub String);

impl FeatureName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FeatureName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for FeatureName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// User objective text describing the goal of the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective(pub String);

impl Objective {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Objective {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Objective {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Working directory path for the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingDir(pub PathBuf);

impl WorkingDir {
    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

impl From<PathBuf> for WorkingDir {
    fn from(p: PathBuf) -> Self {
        Self(p)
    }
}

impl From<&std::path::Path> for WorkingDir {
    fn from(p: &std::path::Path) -> Self {
        Self(p.to_path_buf())
    }
}

/// Absolute path to a plan file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanPath(pub PathBuf);

impl PlanPath {
    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

impl From<PathBuf> for PlanPath {
    fn from(p: PathBuf) -> Self {
        Self(p)
    }
}

/// Absolute path to a feedback file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackPath(pub PathBuf);

impl FeedbackPath {
    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

impl From<PathBuf> for FeedbackPath {
    fn from(p: PathBuf) -> Self {
        Self(p)
    }
}

/// Current iteration number (1-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Iteration(pub u32);

impl Iteration {
    /// Creates a new iteration starting at 1.
    pub fn first() -> Self {
        Self(1)
    }

    /// Increments the iteration and returns the new value.
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl Default for Iteration {
    fn default() -> Self {
        Self::first()
    }
}

/// Maximum allowed iterations for a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaxIterations(pub u32);

impl Default for MaxIterations {
    fn default() -> Self {
        Self(3) // Default to 3 iterations
    }
}

/// Identifier for an agent or reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Identifier for an agent conversation (for resume functionality).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ConversationId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ConversationId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// UTC timestamp for events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampUtc(pub DateTime<Utc>);

impl TimestampUtc {
    /// Creates a timestamp for the current moment.
    pub fn now() -> Self {
        Self(Utc::now())
    }

    /// Returns the timestamp as an RFC3339 string.
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339()
    }
}

impl Default for TimestampUtc {
    fn default() -> Self {
        Self::now()
    }
}

/// Planning workflow phase state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Planning,
    Reviewing,
    Revising,
    AwaitingPlanningDecision,
    Complete,
}

/// Implementation phase state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationPhase {
    #[default]
    Implementing,
    ImplementationReview,
    AwaitingDecision,
    Complete,
}

impl ImplementationPhase {
    /// Returns a human-readable label for this phase.
    pub fn label(&self) -> &'static str {
        match self {
            ImplementationPhase::Implementing => "Implementing",
            ImplementationPhase::ImplementationReview => "Reviewing Implementation",
            ImplementationPhase::AwaitingDecision => "Awaiting Decision",
            ImplementationPhase::Complete => "Implementation Complete",
        }
    }
}

/// UI-friendly phase labels for display purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseLabel {
    Planning,
    Reviewing,
    Revising,
    AwaitingDecision,
    Implementing,
    ImplementationReview,
    ImplementationAwaitingDecision,
    Complete,
}

impl PhaseLabel {
    /// Short label for compact display (e.g., status bars).
    pub fn short(&self) -> &'static str {
        match self {
            PhaseLabel::Planning => "Plan",
            PhaseLabel::Reviewing => "Review",
            PhaseLabel::Revising => "Revise",
            PhaseLabel::AwaitingDecision => "Decide",
            PhaseLabel::Implementing => "Impl",
            PhaseLabel::ImplementationReview => "ImplRev",
            PhaseLabel::ImplementationAwaitingDecision => "ImplDec",
            PhaseLabel::Complete => "Done",
        }
    }

    /// Full label for verbose display.
    pub fn full(&self) -> &'static str {
        match self {
            PhaseLabel::Planning => "Planning",
            PhaseLabel::Reviewing => "Reviewing",
            PhaseLabel::Revising => "Revising",
            PhaseLabel::AwaitingDecision => "Awaiting Decision",
            PhaseLabel::Implementing => "Implementing",
            PhaseLabel::ImplementationReview => "Implementation Review",
            PhaseLabel::ImplementationAwaitingDecision => "Implementation Awaiting Decision",
            PhaseLabel::Complete => "Complete",
        }
    }

    /// Label with iteration number for review/revise phases.
    pub fn with_iteration(&self, iteration: u32) -> String {
        match self {
            PhaseLabel::Reviewing if iteration > 1 => format!("Reviewing #{}", iteration),
            PhaseLabel::Revising => format!("Revising #{}", iteration),
            _ => self.full().to_string(),
        }
    }
}

impl std::fmt::Display for PhaseLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full())
    }
}

/// Resume strategy for agent conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResumeStrategy {
    #[default]
    Stateless,
    ConversationResume,
    ResumeLatest,
}

/// Implementation review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationVerdict {
    Approved,
    NeedsChanges,
}

/// Feedback status from review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Approved,
    NeedsRevision,
}

/// Implementation phase state tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplementationPhaseState {
    pub phase: ImplementationPhase,
    pub iteration: Iteration,
    pub max_iterations: MaxIterations,
    pub last_verdict: Option<ImplementationVerdict>,
    pub last_feedback: Option<String>,
}

impl ImplementationPhaseState {
    /// Creates a new implementation phase state.
    pub fn new(max_iterations: MaxIterations) -> Self {
        Self {
            phase: ImplementationPhase::Implementing,
            iteration: Iteration::first(),
            max_iterations,
            last_verdict: None,
            last_feedback: None,
        }
    }

    /// Returns true if we can continue with another iteration.
    pub fn can_continue(&self) -> bool {
        self.phase != ImplementationPhase::Complete && self.iteration.0 <= self.max_iterations.0
    }

    /// Returns true if the last verdict was Approved.
    pub fn is_approved(&self) -> bool {
        self.last_verdict == Some(ImplementationVerdict::Approved)
    }
}

/// Persisted worktree state for session resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorktreeState {
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub source_branch: Option<String>,
    pub original_dir: PathBuf,
}

/// Agent conversation state for resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConversationState {
    pub resume_strategy: ResumeStrategy,
    pub conversation_id: Option<ConversationId>,
    pub last_used_at: TimestampUtc,
}

/// Invocation history entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub agent: AgentId,
    pub phase: PhaseLabel,
    pub timestamp: TimestampUtc,
    pub conversation_id: Option<ConversationId>,
    pub resume_strategy: ResumeStrategy,
}

/// UI mode for theming and display purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiMode {
    /// Planning workflow is active (planning, reviewing, revising phases)
    #[default]
    Planning,
    /// Implementation workflow is active (implementing, implementation review phases)
    Implementation,
}
