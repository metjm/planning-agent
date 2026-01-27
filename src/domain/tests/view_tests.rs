//! Tests for WorkflowView, particularly current_cycle_reviews tracking.

use super::*;
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentId, FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc,
    WorkingDir,
};
use crate::domain::WorkflowEvent;
use std::path::PathBuf;

fn test_aggregate_id() -> String {
    "550e8400-e29b-41d4-a716-446655440000".to_string()
}

fn workflow_created_event() -> WorkflowEvent {
    WorkflowEvent::WorkflowCreated {
        feature_name: FeatureName::from("test-feature"),
        objective: Objective::from("test objective"),
        working_dir: WorkingDir(PathBuf::from("/test/dir")),
        max_iterations: MaxIterations(3),
        plan_path: PlanPath(PathBuf::from("/test/plan.md")),
        feedback_path: FeedbackPath::from(PathBuf::from("/test/feedback.md")),
        created_at: TimestampUtc::now(),
    }
}

fn review_cycle_started_event() -> WorkflowEvent {
    WorkflowEvent::ReviewCycleStarted {
        mode: ReviewMode::Parallel,
        reviewers: vec![AgentId::from("reviewer-1"), AgentId::from("reviewer-2")],
        started_at: TimestampUtc::now(),
    }
}

fn reviewer_approved_event(reviewer_id: &str) -> WorkflowEvent {
    WorkflowEvent::ReviewerApproved {
        reviewer_id: AgentId::from(reviewer_id),
        approved_at: TimestampUtc::now(),
    }
}

fn reviewer_rejected_event(reviewer_id: &str, feedback_path: &str) -> WorkflowEvent {
    WorkflowEvent::ReviewerRejected {
        reviewer_id: AgentId::from(reviewer_id),
        feedback_path: FeedbackPath::from(PathBuf::from(feedback_path)),
        rejected_at: TimestampUtc::now(),
    }
}

fn revision_completed_event() -> WorkflowEvent {
    WorkflowEvent::RevisionCompleted {
        plan_path: PlanPath(PathBuf::from("/test/plan.md")),
        completed_at: TimestampUtc::now(),
    }
}

#[test]
fn current_cycle_reviews_starts_empty() {
    let view = WorkflowView::default();
    assert!(view.current_cycle_reviews().is_empty());
}

#[test]
fn reviewer_approved_adds_to_current_cycle_reviews() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);

    let reviews = view.current_cycle_reviews();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].reviewer_id.as_str(), "claude");
    assert!(reviews[0].approved);
    assert!(reviews[0].feedback_path.is_none());
}

#[test]
fn reviewer_rejected_adds_to_current_cycle_reviews_with_feedback_path() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("codex", "/test/feedback_1_codex.md"),
        3,
    );

    let reviews = view.current_cycle_reviews();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].reviewer_id.as_str(), "codex");
    assert!(!reviews[0].approved);
    assert!(reviews[0].feedback_path.is_some());
    assert_eq!(
        reviews[0].feedback_path.as_ref().unwrap().as_path(),
        PathBuf::from("/test/feedback_1_codex.md").as_path()
    );
}

#[test]
fn multiple_reviewers_accumulate_in_current_cycle_reviews() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("codex", "/test/feedback_1_codex.md"),
        4,
    );
    view.apply_event(&agg_id, &reviewer_approved_event("gemini"), 5);

    let reviews = view.current_cycle_reviews();
    assert_eq!(reviews.len(), 3);

    // Check claude approved
    assert_eq!(reviews[0].reviewer_id.as_str(), "claude");
    assert!(reviews[0].approved);

    // Check codex rejected with feedback
    assert_eq!(reviews[1].reviewer_id.as_str(), "codex");
    assert!(!reviews[1].approved);
    assert!(reviews[1].feedback_path.is_some());

    // Check gemini approved
    assert_eq!(reviews[2].reviewer_id.as_str(), "gemini");
    assert!(reviews[2].approved);
}

#[test]
fn review_cycle_started_clears_current_cycle_reviews() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);

    assert_eq!(view.current_cycle_reviews().len(), 1);

    // New review cycle starts - should clear previous reviews
    view.apply_event(&agg_id, &review_cycle_started_event(), 4);

    assert!(view.current_cycle_reviews().is_empty());
}

#[test]
fn revision_completed_clears_current_cycle_reviews() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/feedback.md"),
        3,
    );

    assert_eq!(view.current_cycle_reviews().len(), 1);

    // Revision completes - should clear reviews for next cycle
    view.apply_event(&agg_id, &revision_completed_event(), 4);

    assert!(view.current_cycle_reviews().is_empty());
}

#[test]
fn current_cycle_reviews_survives_serialization() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("codex", "/test/feedback.md"),
        4,
    );

    // Serialize and deserialize
    let json = serde_json::to_string(&view).expect("serialize");
    let restored: WorkflowView = serde_json::from_str(&json).expect("deserialize");

    let reviews = restored.current_cycle_reviews();
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[0].reviewer_id.as_str(), "claude");
    assert!(reviews[0].approved);
    assert_eq!(reviews[1].reviewer_id.as_str(), "codex");
    assert!(!reviews[1].approved);
    assert!(reviews[1].feedback_path.is_some());
}

fn max_iterations_extended_event(new_max: u32) -> WorkflowEvent {
    WorkflowEvent::MaxIterationsExtended {
        new_max: MaxIterations(new_max),
        extended_at: TimestampUtc::now(),
    }
}

fn planning_max_iterations_reached_event() -> WorkflowEvent {
    WorkflowEvent::PlanningMaxIterationsReached {
        reached_at: TimestampUtc::now(),
    }
}

fn revising_started_event() -> WorkflowEvent {
    WorkflowEvent::RevisingStarted {
        feedback_summary: "Test feedback".to_string(),
        started_at: TimestampUtc::now(),
    }
}

fn user_declined_event(feedback: &str) -> WorkflowEvent {
    WorkflowEvent::UserDeclined {
        feedback: feedback.to_string(),
        declined_at: TimestampUtc::now(),
    }
}

#[test]
fn max_iterations_extended_updates_view() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);

    // Initial max_iterations should be 3 (from workflow_created_event)
    assert_eq!(view.max_iterations().unwrap().0, 3);

    // Apply MaxIterationsExtended
    view.apply_event(&agg_id, &max_iterations_extended_event(8), 2);

    // max_iterations should now be 8
    assert_eq!(view.max_iterations().unwrap().0, 8);
}

#[test]
fn max_iterations_extended_allows_should_continue() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/feedback.md"),
        3,
    );

    // Simulate reaching max iterations (iteration 3, max 3)
    // First, advance iteration by completing revisions
    view.apply_event(&agg_id, &revision_completed_event(), 4); // iteration now 2
    view.apply_event(&agg_id, &review_cycle_started_event(), 5);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/feedback2.md"),
        6,
    );
    view.apply_event(&agg_id, &revision_completed_event(), 7); // iteration now 3
    view.apply_event(&agg_id, &review_cycle_started_event(), 8);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/feedback3.md"),
        9,
    );
    view.apply_event(&agg_id, &revision_completed_event(), 10); // iteration now 4

    // Now iteration (4) > max_iterations (3), should_continue returns false
    assert!(!view.should_continue());

    // Simulate user extending max_iterations to 6
    view.apply_event(&agg_id, &max_iterations_extended_event(6), 11);

    // Now iteration (4) <= max_iterations (6), should_continue returns true
    // But we're in Reviewing phase after revision_completed, need to check phase
    assert_eq!(
        view.planning_phase(),
        Some(crate::domain::types::Phase::Reviewing)
    );
    assert!(view.should_continue());
}

#[test]
fn full_max_iterations_extension_flow() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    // 1. Create workflow with max_iterations=3
    view.apply_event(&agg_id, &workflow_created_event(), 1);
    assert_eq!(view.max_iterations().unwrap().0, 3);
    assert_eq!(view.iteration().unwrap().0, 1);

    // 2. First review cycle - rejected
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/f1.md"),
        3,
    );

    // 3. Revision completes - iteration advances to 2
    view.apply_event(&agg_id, &revision_completed_event(), 4);
    assert_eq!(view.iteration().unwrap().0, 2);

    // 4. Second review cycle - rejected
    view.apply_event(&agg_id, &review_cycle_started_event(), 5);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/f2.md"),
        6,
    );

    // 5. Revision completes - iteration advances to 3
    view.apply_event(&agg_id, &revision_completed_event(), 7);
    assert_eq!(view.iteration().unwrap().0, 3);

    // 6. Third review cycle - rejected
    view.apply_event(&agg_id, &review_cycle_started_event(), 8);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("claude", "/test/f3.md"),
        9,
    );

    // 7. Max iterations reached
    view.apply_event(&agg_id, &planning_max_iterations_reached_event(), 10);
    assert_eq!(
        view.planning_phase(),
        Some(crate::domain::types::Phase::AwaitingPlanningDecision)
    );

    // 8. User chooses to continue with 5 more iterations
    //    This emits MaxIterationsExtended then RevisingStarted
    view.apply_event(&agg_id, &max_iterations_extended_event(8), 11); // 3 + 5 = 8
    assert_eq!(view.max_iterations().unwrap().0, 8);

    view.apply_event(&agg_id, &revising_started_event(), 12);
    assert_eq!(
        view.planning_phase(),
        Some(crate::domain::types::Phase::Revising)
    );

    // 9. Revision completes - iteration advances to 4
    view.apply_event(&agg_id, &revision_completed_event(), 13);
    assert_eq!(view.iteration().unwrap().0, 4);
    assert_eq!(
        view.planning_phase(),
        Some(crate::domain::types::Phase::Reviewing)
    );

    // 10. Verify should_continue is true (4 <= 8)
    assert!(view.should_continue());
}

// --- User Feedback History Tests ---

#[test]
fn user_feedback_history_starts_empty() {
    let view = WorkflowView::default();
    assert!(view.user_feedback_history().is_empty());
}

#[test]
fn user_declined_accumulates_feedback_in_view() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &user_declined_event("Add more tests"), 2);

    let feedback = view.user_feedback_history();
    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0], "Add more tests");
}

#[test]
fn multiple_user_declined_events_accumulate_in_order() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &user_declined_event("Focus on error handling"), 2);
    view.apply_event(&agg_id, &user_declined_event("Add integration tests"), 3);
    view.apply_event(&agg_id, &user_declined_event("Consider edge cases"), 4);

    let feedback = view.user_feedback_history();
    assert_eq!(feedback.len(), 3);
    assert_eq!(feedback[0], "Focus on error handling");
    assert_eq!(feedback[1], "Add integration tests");
    assert_eq!(feedback[2], "Consider edge cases");
}

#[test]
fn empty_feedback_is_not_accumulated() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &user_declined_event(""), 2);
    view.apply_event(&agg_id, &user_declined_event("Real feedback"), 3);
    view.apply_event(&agg_id, &user_declined_event(""), 4);

    let feedback = view.user_feedback_history();
    assert_eq!(feedback.len(), 1);
    assert_eq!(feedback[0], "Real feedback");
}

#[test]
fn user_feedback_history_survives_serialization() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &user_declined_event("First feedback"), 2);
    view.apply_event(&agg_id, &user_declined_event("Second feedback"), 3);

    // Serialize and deserialize
    let json = serde_json::to_string(&view).expect("serialize");
    let restored: WorkflowView = serde_json::from_str(&json).expect("deserialize");

    let feedback = restored.user_feedback_history();
    assert_eq!(feedback.len(), 2);
    assert_eq!(feedback[0], "First feedback");
    assert_eq!(feedback[1], "Second feedback");
}

#[test]
fn old_sessions_without_user_feedback_history_load_correctly() {
    // Simulate an old session JSON without user_feedback_history field
    // Note: Phase enum uses lowercase variants in JSON (serde(rename_all = "snake_case"))
    let old_json = r#"{
        "workflow_id": "550e8400-e29b-41d4-a716-446655440000",
        "feature_name": "test",
        "objective": "test objective",
        "working_dir": "/test",
        "plan_path": "/test/plan.md",
        "feedback_path": "/test/feedback.md",
        "planning_phase": "planning",
        "iteration": 1,
        "max_iterations": 3,
        "agent_conversations": {},
        "invocations": [],
        "failure_history": [],
        "approval_overridden": false,
        "last_event_sequence": 1,
        "current_cycle_reviews": []
    }"#;

    let view: WorkflowView = serde_json::from_str(old_json).expect("deserialize old session");

    // user_feedback_history should default to empty Vec
    assert!(view.user_feedback_history().is_empty());
}
