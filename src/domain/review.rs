//! Review-related domain types for sequential and parallel review workflows.

use crate::domain::types::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Serializable version of ReviewResult for state persistence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerializableReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
    pub summary: String,
}

/// Sequential review state: tracks progress through reviewer queue
/// and ensures all reviewers approve the same plan version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SequentialReviewState {
    /// Index of the current reviewer in the current cycle order (0-indexed)
    pub current_reviewer_index: usize,
    /// Plan version counter - incremented each time the plan is modified (during revision)
    /// All reviewers must approve the same version for final approval
    pub plan_version: u32,
    /// The plan version that each reviewer last approved (reviewer_display_id -> version)
    pub approvals: HashMap<AgentId, u32>,
    /// Accumulated approved reviews for summary generation.
    #[serde(default)]
    pub accumulated_reviews: Vec<(AgentId, SerializableReviewResult)>,
    /// Total number of review runs per reviewer (reviewer_display_id -> count).
    #[serde(default)]
    pub reviewer_run_counts: HashMap<AgentId, u32>,
    /// The reviewer order for the current cycle (computed at cycle start).
    #[serde(default)]
    pub current_cycle_order: Vec<AgentId>,
    /// The reviewer who rejected the previous plan version.
    #[serde(default)]
    pub last_rejecting_reviewer: Option<AgentId>,
}

impl SequentialReviewState {
    /// Creates a new sequential review state for a fresh review cycle.
    pub fn new() -> Self {
        Self {
            current_reviewer_index: 0,
            plan_version: 1,
            approvals: HashMap::new(),
            accumulated_reviews: Vec::new(),
            reviewer_run_counts: HashMap::new(),
            current_cycle_order: Vec::new(),
            last_rejecting_reviewer: None,
        }
    }

    /// Called when a reviewer approves - records their approval and stores the review.
    pub fn record_approval(&mut self, reviewer_id: &AgentId, review: &SerializableReviewResult) {
        self.approvals
            .insert(reviewer_id.clone(), self.plan_version);
        // Remove any existing review from this reviewer
        self.accumulated_reviews.retain(|(id, _)| id != reviewer_id);
        self.accumulated_reviews
            .push((reviewer_id.clone(), review.clone()));
    }

    /// Called after revision - increments version and clears stale approvals.
    pub fn increment_version(&mut self) {
        self.plan_version += 1;
        self.approvals.clear();
        self.accumulated_reviews.clear();
    }

    /// Checks if all reviewers have approved the current plan version.
    pub fn all_approved(&self, reviewer_ids: &[&AgentId]) -> bool {
        reviewer_ids
            .iter()
            .all(|id| self.approvals.get(*id) == Some(&self.plan_version))
    }

    /// Resets to first reviewer for a new cycle.
    pub fn reset_to_first_reviewer(&mut self) {
        self.current_reviewer_index = 0;
        self.current_cycle_order.clear();
    }

    /// Advances to next reviewer.
    pub fn advance_to_next_reviewer(&mut self) {
        self.current_reviewer_index += 1;
    }

    /// Increments the run count for a reviewer.
    pub fn increment_run_count(&mut self, reviewer_id: &AgentId) {
        *self
            .reviewer_run_counts
            .entry(reviewer_id.clone())
            .or_insert(0) += 1;
    }

    /// Returns the run count for a reviewer (0 if never run).
    pub fn get_run_count(&self, reviewer_id: &AgentId) -> u32 {
        self.reviewer_run_counts
            .get(reviewer_id)
            .copied()
            .unwrap_or(0)
    }

    /// Records which reviewer rejected the plan.
    pub fn record_rejection(&mut self, reviewer_id: &AgentId) {
        self.last_rejecting_reviewer = Some(reviewer_id.clone());
    }

    /// Starts a new review cycle by computing and storing the sorted reviewer order.
    pub fn start_new_cycle(&mut self, reviewer_ids: &[&AgentId]) -> Option<AgentId> {
        let mut sorted: Vec<AgentId> = reviewer_ids.iter().map(|s| (*s).clone()).collect();
        let last_rejector = self.last_rejecting_reviewer.take();
        let tiebreaker_used = last_rejector.clone();

        sorted.sort_by(|a, b| {
            let count_a = self.get_run_count(a);
            let count_b = self.get_run_count(b);

            match count_a.cmp(&count_b) {
                std::cmp::Ordering::Equal => match (&last_rejector, a, b) {
                    (Some(rejector), a, _) if a == rejector => std::cmp::Ordering::Less,
                    (Some(rejector), _, b) if b == rejector => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                },
                other => other,
            }
        });

        self.current_cycle_order = sorted;
        self.current_reviewer_index = 0;

        tiebreaker_used
    }

    /// Gets the current reviewer's ID from the stored cycle order.
    pub fn get_current_reviewer(&self) -> Option<&AgentId> {
        self.current_cycle_order.get(self.current_reviewer_index)
    }

    /// Returns true if the cycle order needs to be (re)computed.
    pub fn needs_cycle_start(&self) -> bool {
        self.current_cycle_order.is_empty()
    }
}

/// Converts from the state module's SequentialReviewState to the domain version.
impl From<crate::state::SequentialReviewState> for SequentialReviewState {
    fn from(state: crate::state::SequentialReviewState) -> Self {
        Self {
            current_reviewer_index: state.current_reviewer_index,
            plan_version: state.plan_version,
            approvals: state
                .approvals
                .into_iter()
                .map(|(k, v)| (AgentId::from(k), v))
                .collect(),
            accumulated_reviews: state
                .accumulated_reviews
                .into_iter()
                .map(|(id, review)| {
                    (
                        AgentId::from(id),
                        SerializableReviewResult {
                            agent_name: review.agent_name,
                            needs_revision: review.needs_revision,
                            feedback: review.feedback,
                            summary: review.summary,
                        },
                    )
                })
                .collect(),
            reviewer_run_counts: state
                .reviewer_run_counts
                .into_iter()
                .map(|(k, v)| (AgentId::from(k), v))
                .collect(),
            current_cycle_order: state
                .current_cycle_order
                .into_iter()
                .map(AgentId::from)
                .collect(),
            last_rejecting_reviewer: state.last_rejecting_reviewer.map(AgentId::from),
        }
    }
}

/// Review mode for the workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Parallel,
    Sequential(SequentialReviewState),
}
