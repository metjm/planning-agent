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
