//! Review-related domain types for sequential and parallel review workflows.
//!
//! IMPORTANT: These types are part of the domain model. State mutations MUST only
//! happen through the aggregate's event handlers. Mutation methods are `pub(crate)`
//! to prevent external code from bypassing the CQRS pattern.

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
///
/// # Invariants
/// - All mutations happen through the aggregate's event handlers
/// - External code can only read via getter methods
/// - Fields are private to enforce this
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SequentialReviewState {
    /// Index of the current reviewer in the current cycle order (0-indexed)
    current_reviewer_index: usize,
    /// Plan version counter - incremented each time the plan is modified (during revision)
    /// All reviewers must approve the same version for final approval
    plan_version: u32,
    /// The plan version that each reviewer last approved (reviewer_display_id -> version)
    approvals: HashMap<AgentId, u32>,
    /// Accumulated approved reviews for summary generation.
    #[serde(default)]
    accumulated_reviews: Vec<(AgentId, SerializableReviewResult)>,
    /// The reviewer order for the current cycle (computed at cycle start).
    #[serde(default)]
    current_cycle_order: Vec<AgentId>,
    /// The reviewer who rejected the previous plan version.
    #[serde(default)]
    last_rejecting_reviewer: Option<AgentId>,
}

impl SequentialReviewState {
    /// Creates a new sequential review state for a fresh review cycle.
    pub fn new() -> Self {
        Self {
            current_reviewer_index: 0,
            plan_version: 1,
            approvals: HashMap::new(),
            accumulated_reviews: Vec::new(),
            current_cycle_order: Vec::new(),
            last_rejecting_reviewer: None,
        }
    }

    /// Creates a new sequential review state with the cycle order initialized.
    /// This is the public factory for creating state to pass to ReviewCycleStarted.
    /// The cycle order is computed based on the provided reviewer IDs.
    pub fn new_with_cycle(reviewer_ids: &[&str]) -> Self {
        let mut state = Self::new();
        state.start_new_cycle(reviewer_ids);
        state
    }

    // ========================================================================
    // READ-ONLY GETTERS (public - safe for external code to call)
    // ========================================================================

    /// Returns the current reviewer index.
    pub fn current_reviewer_index(&self) -> usize {
        self.current_reviewer_index
    }

    /// Returns the current plan version.
    pub fn plan_version(&self) -> u32 {
        self.plan_version
    }

    /// Gets the current reviewer's ID from the stored cycle order.
    pub fn get_current_reviewer(&self) -> Option<&AgentId> {
        self.current_cycle_order.get(self.current_reviewer_index)
    }

    /// Returns true if the cycle order needs to be (re)computed.
    pub fn needs_cycle_start(&self) -> bool {
        self.current_cycle_order.is_empty()
    }

    /// Checks if all reviewers have approved the current plan version.
    pub fn all_approved(&self, reviewer_ids: &[&str]) -> bool {
        reviewer_ids.iter().all(|id| {
            let agent_id = AgentId::from(*id);
            self.approvals.get(&agent_id) == Some(&self.plan_version)
        })
    }

    /// Returns the current cycle order (for logging/display).
    pub fn cycle_order(&self) -> &[AgentId] {
        &self.current_cycle_order
    }

    /// Returns the approvals map (reviewer -> plan version they approved).
    pub fn approvals(&self) -> &HashMap<AgentId, u32> {
        &self.approvals
    }

    /// Returns the last rejecting reviewer (if any).
    pub fn last_rejecting_reviewer(&self) -> Option<&AgentId> {
        self.last_rejecting_reviewer.as_ref()
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

    // ========================================================================
    // MUTATION METHODS (pub(crate) - only callable from domain module)
    // These are called by the aggregate's apply() method in response to events.
    // ========================================================================

    /// Records a reviewer approval without storing the review content.
    /// Used by aggregate event handler when review content isn't available.
    /// ONLY call from aggregate event handlers.
    pub(crate) fn record_approval_simple(&mut self, reviewer_id: AgentId) {
        self.approvals.insert(reviewer_id, self.plan_version);
    }

    /// Called after revision - increments version and clears stale approvals.
    /// ONLY call from aggregate event handlers.
    pub(crate) fn increment_version(&mut self) {
        self.plan_version += 1;
        self.approvals.clear();
        self.accumulated_reviews.clear();
    }

    /// Advances to next reviewer.
    /// ONLY call from aggregate event handlers.
    pub(crate) fn advance_to_next_reviewer(&mut self) {
        self.current_reviewer_index += 1;
    }

    /// Records which reviewer rejected the plan.
    /// ONLY call from aggregate event handlers.
    pub(crate) fn record_rejection(&mut self, reviewer_id: &str) {
        self.last_rejecting_reviewer = Some(AgentId::from(reviewer_id));
    }

    /// Starts a new review cycle by computing and storing the reviewer order.
    /// The last rejecting reviewer (if any) is prioritized first.
    /// ONLY call from aggregate event handlers.
    pub(crate) fn start_new_cycle(&mut self, reviewer_ids: &[&str]) -> Option<AgentId> {
        let mut sorted: Vec<AgentId> = reviewer_ids.iter().map(|s| AgentId::from(*s)).collect();
        let last_rejector = self.last_rejecting_reviewer.take();
        let tiebreaker_used = last_rejector.clone();

        // Sort by tiebreaker: prioritize the last rejecting reviewer
        if let Some(ref rejector) = last_rejector {
            sorted.sort_by(|a, b| {
                if a == rejector {
                    std::cmp::Ordering::Less
                } else if b == rejector {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
        }

        self.current_cycle_order = sorted;
        self.current_reviewer_index = 0;

        tiebreaker_used
    }

    /// Clears the cycle order (called after revision).
    /// ONLY call from aggregate event handlers.
    pub(crate) fn clear_cycle_order(&mut self) {
        self.current_cycle_order.clear();
    }
}

/// Review mode for the workflow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Parallel,
    Sequential(SequentialReviewState),
}
