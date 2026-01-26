use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, Iteration, MaxIterations, Objective, Phase, PlanPath, WorkflowId,
    WorkingDir,
};
use crate::domain::view::WorkflowView;
use std::path::PathBuf;
use uuid::Uuid;

fn minimal_view() -> WorkflowView {
    WorkflowView {
        workflow_id: Some(WorkflowId(Uuid::new_v4())),
        feature_name: Some(FeatureName("test-feature".to_string())),
        objective: Some(Objective("Test objective".to_string())),
        working_dir: Some(WorkingDir(PathBuf::from("/tmp/workspace"))),
        plan_path: Some(PlanPath(PathBuf::from("/tmp/test-plan/plan.md"))),
        feedback_path: Some(FeedbackPath(PathBuf::from("/tmp/test-feedback.md"))),
        planning_phase: Some(Phase::Complete),
        iteration: Some(Iteration(1)),
        max_iterations: Some(MaxIterations(3)),
        ..Default::default()
    }
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
