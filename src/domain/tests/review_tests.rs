//! Tests for round-robin reviewer ordering and invocation counting.

use super::*;
use crate::domain::types::{AgentId, InvocationRecord, PhaseLabel, ResumeStrategy, TimestampUtc};
use std::collections::HashMap;

/// Helper to create an InvocationRecord with specified agent and phase.
fn make_invocation(agent_id: &str, phase: PhaseLabel) -> InvocationRecord {
    InvocationRecord::new(
        AgentId::from(agent_id),
        phase,
        TimestampUtc::now(),
        None,
        ResumeStrategy::Stateless,
    )
}

// =============================================================================
// count_reviewing_invocations tests
// =============================================================================

#[test]
fn count_reviewing_invocations_strips_namespace_prefix() {
    // Setup: Create invocation records with NAMESPACED agent IDs (as stored in InvocationRecord)
    let invocations = vec![
        make_invocation("reviewing/claude-completeness", PhaseLabel::Reviewing),
        make_invocation("reviewing/claude-completeness", PhaseLabel::Reviewing),
        make_invocation("reviewing/claude-completeness", PhaseLabel::Reviewing),
        make_invocation("reviewing/gemini-consistency", PhaseLabel::Reviewing),
        make_invocation("reviewing/gpt-correctness", PhaseLabel::Reviewing),
        make_invocation("reviewing/gpt-correctness", PhaseLabel::Reviewing),
    ];

    let counts = count_reviewing_invocations(&invocations);

    // Assert counts are keyed by RAW display IDs (prefix stripped)
    assert_eq!(
        counts.get(&AgentId::from("claude-completeness")),
        Some(&3),
        "claude-completeness should have 3 invocations"
    );
    assert_eq!(
        counts.get(&AgentId::from("gemini-consistency")),
        Some(&1),
        "gemini-consistency should have 1 invocation"
    );
    assert_eq!(
        counts.get(&AgentId::from("gpt-correctness")),
        Some(&2),
        "gpt-correctness should have 2 invocations"
    );
}

#[test]
fn count_reviewing_invocations_excludes_non_reviewing_phases() {
    let invocations = vec![
        make_invocation("reviewing/claude", PhaseLabel::Reviewing),
        make_invocation("reviewing/claude", PhaseLabel::Planning), // Should be ignored
        make_invocation("reviewing/claude", PhaseLabel::Revising), // Should be ignored
        make_invocation("reviewing/gemini", PhaseLabel::Reviewing),
        make_invocation("reviewing/gemini", PhaseLabel::AwaitingDecision), // Should be ignored
    ];

    let counts = count_reviewing_invocations(&invocations);

    assert_eq!(counts.get(&AgentId::from("claude")), Some(&1));
    assert_eq!(counts.get(&AgentId::from("gemini")), Some(&1));
}

#[test]
fn count_reviewing_invocations_handles_non_namespaced_ids() {
    // Edge case: InvocationRecord without "reviewing/" prefix (shouldn't happen normally)
    let invocations = vec![
        make_invocation("claude-completeness", PhaseLabel::Reviewing), // No prefix
    ];

    let counts = count_reviewing_invocations(&invocations);

    // strip_prefix returns original if no match, so this should still work
    assert_eq!(counts.get(&AgentId::from("claude-completeness")), Some(&1));
}

#[test]
fn count_reviewing_invocations_empty_list() {
    let counts = count_reviewing_invocations(&[]);
    assert!(counts.is_empty());
}

// =============================================================================
// start_new_cycle round-robin ordering tests
// =============================================================================

#[test]
fn start_new_cycle_orders_by_review_count_ascending() {
    let mut state = SequentialReviewState::new();
    let reviewer_ids = &["agent-a", "agent-b", "agent-c"];
    let mut review_counts: HashMap<AgentId, usize> = HashMap::new();
    review_counts.insert(AgentId::from("agent-a"), 3); // Most reviews
    review_counts.insert(AgentId::from("agent-b"), 1); // Fewest reviews
    review_counts.insert(AgentId::from("agent-c"), 2); // Middle

    state.start_new_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], AgentId::from("agent-b"), "Fewest reviews first");
    assert_eq!(order[1], AgentId::from("agent-c"), "Middle reviews second");
    assert_eq!(order[2], AgentId::from("agent-a"), "Most reviews last");
}

#[test]
fn start_new_cycle_last_rejector_wins_ties() {
    let mut state = SequentialReviewState::new();
    // Set up a last rejecting reviewer
    state.record_rejection("agent-b");

    let reviewer_ids = &["agent-a", "agent-b", "agent-c"];
    let mut review_counts: HashMap<AgentId, usize> = HashMap::new();
    // All have same count (tie)
    review_counts.insert(AgentId::from("agent-a"), 2);
    review_counts.insert(AgentId::from("agent-b"), 2);
    review_counts.insert(AgentId::from("agent-c"), 2);

    state.start_new_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    assert_eq!(order[0], AgentId::from("agent-b"), "Last rejector wins tie");
}

#[test]
fn start_new_cycle_mixed_counts_and_tiebreaker() {
    let mut state = SequentialReviewState::new();
    // B is last rejector
    state.record_rejection("agent-b");

    let reviewer_ids = &["agent-a", "agent-b", "agent-c"];
    let mut review_counts: HashMap<AgentId, usize> = HashMap::new();
    // A and B have 1 review each (tie), C has 2
    review_counts.insert(AgentId::from("agent-a"), 1);
    review_counts.insert(AgentId::from("agent-b"), 1);
    review_counts.insert(AgentId::from("agent-c"), 2);

    state.start_new_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    // B wins tie with A (last rejector), C goes last (higher count)
    assert_eq!(order[0], AgentId::from("agent-b"), "B wins tie as rejector");
    assert_eq!(order[1], AgentId::from("agent-a"), "A second");
    assert_eq!(order[2], AgentId::from("agent-c"), "C last due to count");
}

#[test]
fn start_new_cycle_zero_counts_preserves_config_order() {
    let mut state = SequentialReviewState::new();
    let reviewer_ids = &["agent-a", "agent-b", "agent-c"];
    let review_counts: HashMap<AgentId, usize> = HashMap::new(); // Empty - all have 0

    state.start_new_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    // All have same count (0), stable sort preserves config order
    assert_eq!(order[0], AgentId::from("agent-a"));
    assert_eq!(order[1], AgentId::from("agent-b"));
    assert_eq!(order[2], AgentId::from("agent-c"));
}

#[test]
fn start_new_cycle_missing_counts_treated_as_zero() {
    let mut state = SequentialReviewState::new();
    let reviewer_ids = &["agent-a", "agent-b", "agent-c"];
    let mut review_counts: HashMap<AgentId, usize> = HashMap::new();
    // Only A has counts, B and C are missing (treated as 0)
    review_counts.insert(AgentId::from("agent-a"), 2);

    state.start_new_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    // B and C have 0 (missing), A has 2
    // B and C should come first (stable order), A last
    assert_eq!(order[0], AgentId::from("agent-b"));
    assert_eq!(order[1], AgentId::from("agent-c"));
    assert_eq!(order[2], AgentId::from("agent-a"));
}

// =============================================================================
// new_with_cycle tests
// =============================================================================

#[test]
fn new_with_cycle_initializes_with_review_counts() {
    let reviewer_ids = &["agent-a", "agent-b"];
    let mut review_counts: HashMap<AgentId, usize> = HashMap::new();
    review_counts.insert(AgentId::from("agent-a"), 5);
    review_counts.insert(AgentId::from("agent-b"), 1);

    let state = SequentialReviewState::new_with_cycle(reviewer_ids, &review_counts);

    let order = state.cycle_order();
    assert_eq!(order[0], AgentId::from("agent-b"), "Fewer reviews first");
    assert_eq!(order[1], AgentId::from("agent-a"), "More reviews last");
    assert_eq!(state.current_reviewer_index(), 0);
}
