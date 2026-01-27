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
            max_iterations: MaxIterations::default(),
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
fn test_build_implementation_prompt_basic() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_prompt(&view, &working_dir, 1, None);

    // Check paths are included
    assert!(prompt.contains("/tmp/workspace"));
    assert!(prompt.contains("/tmp/test-plan/plan.md"));

    // Check iteration
    assert!(prompt.contains("IMPLEMENTATION #1"));

    // Check skill instruction is last
    assert!(prompt.ends_with(r#"Run the "implementation" skill to execute the plan."#));
}

#[test]
fn test_build_implementation_prompt_with_feedback() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let feedback = "Missing error handling in src/main.rs";
    let prompt = build_implementation_prompt(&view, &working_dir, 2, Some(feedback));

    // Should include the feedback section
    assert!(prompt.contains("FEEDBACK FROM REVIEW"));
    assert!(prompt.contains("Missing error handling"));

    // Iteration should be 2
    assert!(prompt.contains("IMPLEMENTATION #2"));

    // Skill instruction still last
    assert!(prompt.ends_with(r#"Run the "implementation" skill to execute the plan."#));
}

#[test]
fn test_build_implementation_followup_prompt() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_followup_prompt(&view, &working_dir, "Fix the bug");

    assert!(prompt.contains("USER MESSAGE"));
    assert!(prompt.contains("Fix the bug"));
    assert!(prompt.contains("/tmp/workspace"));
}
