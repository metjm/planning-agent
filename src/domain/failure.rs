//! Structured failure handling types for the workflow domain.
//!
//! This module provides a canonical failure taxonomy for agent-level failures
//! (network, timeout, non-zero exit, parse errors) and workflow-level failures.

use crate::domain::types::{AgentId, PhaseLabel, TimestampUtc};
use serde::{Deserialize, Serialize};

/// Maximum number of failure records to keep in history to prevent unbounded growth.
pub const MAX_FAILURE_HISTORY: usize = 50;

/// Canonical failure types for agent and workflow failures.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Activity timeout - no output for configured duration.
    Timeout,
    /// Network-related error detected from stderr patterns.
    Network,
    /// Non-zero exit code from agent process.
    ProcessExit(i32),
    /// Output parsing failed with the given error message.
    ParseFailure(String),
    /// Agent produced no output.
    EmptyOutput,
    /// Workflow-level failure when no reviews completed.
    AllReviewersFailed,
    /// Unclassified errors for future extensibility.
    Unknown(String),
}

impl FailureKind {
    /// Returns true if this failure type is potentially recoverable via retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            FailureKind::Timeout
                | FailureKind::Network
                | FailureKind::EmptyOutput
                | FailureKind::AllReviewersFailed
        )
    }

    /// Returns a human-readable name for this failure type.
    pub fn display_name(&self) -> &'static str {
        match self {
            FailureKind::Timeout => "Timeout",
            FailureKind::Network => "Network",
            FailureKind::ProcessExit(_) => "Process Exit",
            FailureKind::ParseFailure(_) => "Parse Failure",
            FailureKind::EmptyOutput => "Empty Output",
            FailureKind::AllReviewersFailed => "All Reviewers Failed",
            FailureKind::Unknown(_) => "Unknown",
        }
    }
}

/// Actions that can be taken to recover from a failure.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// User chose to retry the failed operation.
    Retried,
    /// User chose to stop and save state for later resume.
    Stopped,
    /// User chose to abort the workflow.
    Aborted,
    /// User chose to continue without full review (partial reviews available).
    ContinuedWithoutFullReview,
}

/// Context for a workflow failure, persisted in state for recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FailureContext {
    /// Classified failure type.
    pub kind: FailureKind,
    /// Which phase the failure occurred in.
    pub phase: PhaseLabel,
    /// Which agent failed (if agent-level failure).
    pub agent_name: Option<AgentId>,
    /// Number of retries attempted for this failure.
    pub retry_count: u32,
    /// Maximum retries allowed from policy.
    pub max_retries: u32,
    /// Timestamp when failure occurred.
    pub failed_at: TimestampUtc,
    /// How the failure was recovered (set after user decision).
    pub recovery_action: Option<RecoveryAction>,
}

impl FailureContext {
    /// Creates a new FailureContext with the given parameters.
    pub fn new(
        kind: FailureKind,
        phase: PhaseLabel,
        agent_name: Option<AgentId>,
        max_retries: u32,
    ) -> Self {
        Self {
            kind,
            phase,
            agent_name,
            retry_count: 0,
            max_retries,
            failed_at: TimestampUtc::now(),
            recovery_action: None,
        }
    }

    /// Returns true if this failure can be retried based on retry_count and max_retries.
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries && self.kind.is_retryable()
    }

    /// Increments the retry count and updates the failed_at timestamp.
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
        self.failed_at = TimestampUtc::now();
    }

    /// Sets the recovery action taken.
    pub fn set_recovery_action(&mut self, action: RecoveryAction) {
        self.recovery_action = Some(action);
    }
}

/// Policy action when all reviewers fail after retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnAllReviewersFailed {
    /// Stop workflow with error (default)
    #[default]
    Abort,
    /// Save state for later recovery in TUI mode
    SaveState,
    /// Proceed to revision phase without reviews (only if partial reviews exist)
    ContinueWithoutReview,
}

/// Retry policy configuration for failure handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePolicy {
    /// Maximum retry attempts for transient failures. Default: 2
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Backoff multiplier in seconds for retries. Default: 5
    #[serde(default = "default_backoff_secs")]
    pub backoff_secs: u32,
    /// Action when all reviewers fail after retries
    #[serde(default)]
    pub on_all_reviewers_failed: OnAllReviewersFailed,
}

fn default_max_retries() -> u32 {
    2
}

fn default_backoff_secs() -> u32 {
    5
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            backoff_secs: default_backoff_secs(),
            on_all_reviewers_failed: OnAllReviewersFailed::default(),
        }
    }
}

impl FailurePolicy {
    /// Validates the policy configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Regex patterns for classifying network errors from stderr.
/// These patterns are used to identify network-related failures.
pub const NETWORK_ERROR_PATTERN: &str =
    r"(?i)connect|network|ECONNREFUSED|ETIMEDOUT|connection\s+refused|name\s+resolution|DNS|socket";
