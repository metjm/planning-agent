//! Tests for workflow revising module, particularly populate_reviews_from_view.

use crate::app::workflow::revising::populate_reviews_from_view;
use crate::domain::review::ReviewMode;
use crate::domain::types::{
    AgentId, FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc,
    WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use crate::phases::ReviewResult;
use crate::session_daemon::{create_session_logger, SessionLogger};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

fn test_aggregate_id() -> String {
    "550e8400-e29b-41d4-a716-446655440000".to_string()
}

fn create_test_logger() -> Arc<SessionLogger> {
    let session_id = format!("test-{}", uuid::Uuid::new_v4());
    create_session_logger(&session_id).expect("create test logger")
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

fn reviewer_rejected_event(reviewer_id: &str, feedback_path: PathBuf) -> WorkflowEvent {
    WorkflowEvent::ReviewerRejected {
        reviewer_id: AgentId::from(reviewer_id),
        feedback_path: FeedbackPath::from(feedback_path),
        rejected_at: TimestampUtc::now(),
    }
}

#[test]
fn populate_reviews_does_nothing_when_last_reviews_not_empty() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);

    // Pre-populate last_reviews
    let mut last_reviews = vec![ReviewResult {
        agent_name: "existing".to_string(),
        needs_revision: false,
        feedback: "existing feedback".to_string(),
        summary: "existing summary".to_string(),
    }];

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    // Should not have changed
    assert_eq!(last_reviews.len(), 1);
    assert_eq!(last_reviews[0].agent_name, "existing");
}

#[test]
fn populate_reviews_does_nothing_when_view_has_no_reviews() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    // No review cycle started, so no reviews

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert!(last_reviews.is_empty());
}

#[test]
fn populate_reviews_loads_approved_review_from_view() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert_eq!(last_reviews.len(), 1);
    assert_eq!(last_reviews[0].agent_name, "claude");
    assert!(!last_reviews[0].needs_revision); // approved = not needs_revision
}

#[test]
fn populate_reviews_loads_rejected_review_with_feedback_file() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let feedback_path = temp_dir.path().join("feedback_1_codex.md");
    std::fs::write(
        &feedback_path,
        "# Review Feedback\n\nThis plan needs work.\n\n## Issues\n- Issue 1\n- Issue 2",
    )
    .expect("write feedback");

    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_rejected_event("codex", feedback_path), 3);

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert_eq!(last_reviews.len(), 1);
    assert_eq!(last_reviews[0].agent_name, "codex");
    assert!(last_reviews[0].needs_revision); // rejected = needs_revision
    assert!(last_reviews[0].feedback.contains("This plan needs work"));
    assert_eq!(last_reviews[0].summary, "# Review Feedback"); // First non-empty line
}

#[test]
fn populate_reviews_handles_missing_feedback_file() {
    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("codex", PathBuf::from("/nonexistent/feedback.md")),
        3,
    );

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert_eq!(last_reviews.len(), 1);
    assert_eq!(last_reviews[0].agent_name, "codex");
    assert!(last_reviews[0].needs_revision);
    // Should have a placeholder indicating the file path
    assert!(last_reviews[0]
        .feedback
        .contains("/nonexistent/feedback.md"));
}

#[test]
fn populate_reviews_loads_multiple_reviewers() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let codex_feedback = temp_dir.path().join("feedback_1_codex.md");
    std::fs::write(&codex_feedback, "Codex feedback content").expect("write feedback");

    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_approved_event("claude"), 3);
    view.apply_event(
        &agg_id,
        &reviewer_rejected_event("codex", codex_feedback),
        4,
    );
    view.apply_event(&agg_id, &reviewer_approved_event("gemini"), 5);

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert_eq!(last_reviews.len(), 3);

    // Check claude (approved)
    assert_eq!(last_reviews[0].agent_name, "claude");
    assert!(!last_reviews[0].needs_revision);

    // Check codex (rejected with feedback)
    assert_eq!(last_reviews[1].agent_name, "codex");
    assert!(last_reviews[1].needs_revision);
    assert!(last_reviews[1].feedback.contains("Codex feedback"));

    // Check gemini (approved)
    assert_eq!(last_reviews[2].agent_name, "gemini");
    assert!(!last_reviews[2].needs_revision);
}

#[test]
fn populate_reviews_truncates_long_summary() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let feedback_path = temp_dir.path().join("feedback.md");
    // Create a feedback file with a very long first line (> 100 chars)
    let long_line = "A".repeat(150);
    std::fs::write(&feedback_path, &long_line).expect("write feedback");

    let mut view = WorkflowView::default();
    let agg_id = test_aggregate_id();
    let logger = create_test_logger();

    view.apply_event(&agg_id, &workflow_created_event(), 1);
    view.apply_event(&agg_id, &review_cycle_started_event(), 2);
    view.apply_event(&agg_id, &reviewer_rejected_event("codex", feedback_path), 3);

    let mut last_reviews: Vec<ReviewResult> = Vec::new();

    populate_reviews_from_view(&view, &mut last_reviews, &*logger);

    assert_eq!(last_reviews.len(), 1);
    // Summary should be truncated to ~100 chars with "..."
    assert!(last_reviews[0].summary.len() <= 103); // 97 + "..."
    assert!(last_reviews[0].summary.ends_with("..."));
}
