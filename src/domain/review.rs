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
    pub fn record_approval(&mut self, reviewer_id: &str, review: &SerializableReviewResult) {
        let agent_id = AgentId::from(reviewer_id);
        self.approvals.insert(agent_id.clone(), self.plan_version);
        // Remove any existing review from this reviewer
        self.accumulated_reviews
            .retain(|(id, _)| id.as_str() != reviewer_id);
        self.accumulated_reviews.push((agent_id, review.clone()));
    }

    /// Called after revision - increments version and clears stale approvals.
    pub fn increment_version(&mut self) {
        self.plan_version += 1;
        self.approvals.clear();
        self.accumulated_reviews.clear();
    }

    /// Checks if all reviewers have approved the current plan version.
    pub fn all_approved(&self, reviewer_ids: &[&str]) -> bool {
        reviewer_ids.iter().all(|id| {
            let agent_id = AgentId::from(*id);
            self.approvals.get(&agent_id) == Some(&self.plan_version)
        })
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
    pub fn increment_run_count(&mut self, reviewer_id: &str) {
        let agent_id = AgentId::from(reviewer_id);
        *self.reviewer_run_counts.entry(agent_id).or_insert(0) += 1;
    }

    /// Returns the run count for a reviewer (0 if never run).
    pub fn get_run_count(&self, reviewer_id: &str) -> u32 {
        let agent_id = AgentId::from(reviewer_id);
        self.reviewer_run_counts
            .get(&agent_id)
            .copied()
            .unwrap_or(0)
    }

    /// Records which reviewer rejected the plan.
    pub fn record_rejection(&mut self, reviewer_id: &str) {
        self.last_rejecting_reviewer = Some(AgentId::from(reviewer_id));
    }

    /// Starts a new review cycle by computing and storing the sorted reviewer order.
    pub fn start_new_cycle(&mut self, reviewer_ids: &[&str]) -> Option<AgentId> {
        let mut sorted: Vec<AgentId> = reviewer_ids.iter().map(|s| AgentId::from(*s)).collect();
        let last_rejector = self.last_rejecting_reviewer.take();
        let tiebreaker_used = last_rejector.clone();

        sorted.sort_by(|a, b| {
            let count_a = self.get_run_count(a.as_str());
            let count_b = self.get_run_count(b.as_str());

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

    /// Validates that the current state is consistent with the given reviewer IDs.
    /// Returns true if state was reset (config changed), false if valid.
    pub fn validate_reviewer_state(&mut self, reviewer_ids: &[&str]) -> bool {
        // Check if stored cycle order contains reviewers not in current config
        let current_ids: std::collections::HashSet<&str> = reviewer_ids.iter().copied().collect();
        let stored_ids: std::collections::HashSet<&str> = self
            .current_cycle_order
            .iter()
            .map(|id| id.as_str())
            .collect();

        // If cycle order is non-empty and doesn't match config, reset
        if !self.current_cycle_order.is_empty() && stored_ids != current_ids {
            self.reset_to_first_reviewer();
            return true;
        }
        false
    }

    /// Returns accumulated reviews as ReviewResult references for summary generation.
    pub fn get_accumulated_reviews_for_summary(&self) -> Vec<crate::phases::ReviewResult> {
        self.accumulated_reviews
            .iter()
            .map(|(_, r)| crate::phases::ReviewResult {
                agent_name: r.agent_name.clone(),
                needs_revision: r.needs_revision,
                feedback: r.feedback.clone(),
                summary: r.summary.clone(),
            })
            .collect()
    }
}

/// Review mode for the workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Parallel,
    Sequential(SequentialReviewState),
}
