use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use std::path::PathBuf;
use uuid::Uuid;

fn minimal_view() -> WorkflowView {
    let mut view = WorkflowView::default();
    let agg_id = Uuid::new_v4().to_string();

    // Create workflow
    view.apply_event(
        &agg_id,
        &WorkflowEvent::WorkflowCreated {
            feature_name: FeatureName::from("test-feature"),
            objective: Objective::from("Test objective"),
            working_dir: WorkingDir::from(PathBuf::from("/tmp/workspace").as_path()),
            max_iterations: MaxIterations(3),
            plan_path: PlanPath::from(PathBuf::from("/tmp/test-plan/plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test-feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    // Move to Complete phase
    view.apply_event(
        &agg_id,
        &WorkflowEvent::PlanningCompleted {
            plan_path: PlanPath::from(PathBuf::from("/tmp/test-plan/plan.md")),
            completed_at: TimestampUtc::now(),
        },
        2,
    );
    view.apply_event(
        &agg_id,
        &WorkflowEvent::ReviewCycleCompleted {
            approved: true,
            completed_at: TimestampUtc::now(),
        },
        3,
    );

    view
}

#[test]
fn test_build_implementation_review_prompt_basic() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_review_prompt(&view, &working_dir, 1, None)
        .expect("build_implementation_review_prompt failed");

    // Check paths are included
    assert!(prompt.contains("/tmp/workspace"));
    assert!(prompt.contains("/tmp/test-plan/plan.md"));

    // Check iteration
    assert!(prompt.contains("IMPLEMENTATION REVIEW #1"));

    // Check skill instruction is last
    assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
}

#[test]
fn test_build_implementation_review_prompt_with_log_path() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let log_path = PathBuf::from("/tmp/session/implementation_1.log");
    let prompt = build_implementation_review_prompt(&view, &working_dir, 1, Some(&log_path))
        .expect("build_implementation_review_prompt failed");

    // Should include the implementation log path
    assert!(prompt.contains("Implementation log:"));
    assert!(prompt.contains("implementation_1.log"));

    // Skill instruction still last
    assert!(prompt.ends_with(r#"Run the "implementation-review" skill to perform the review."#));
}

#[test]
fn test_implementation_review_result_with_needs_revision_verdict() {
    let result = ImplementationReviewResult {
        verdict: VerificationVerdictResult::NeedsRevision,
        feedback: Some("Fix this".to_string()),
    };

    assert!(result.verdict.needs_revision());
    assert_eq!(result.feedback, Some("Fix this".to_string()));
}
