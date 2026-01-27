use super::*;
use crate::domain::types::{
    FeatureName, FeedbackPath, MaxIterations, Objective, PlanPath, TimestampUtc, WorkingDir,
};
use crate::domain::view::WorkflowView;
use crate::domain::WorkflowEvent;
use std::path::PathBuf;
use uuid::Uuid;

fn user_declined_event(feedback: &str) -> WorkflowEvent {
    WorkflowEvent::UserDeclined {
        feedback: feedback.to_string(),
        declined_at: TimestampUtc::now(),
    }
}

fn minimal_view() -> WorkflowView {
    let mut view = WorkflowView::default();
    let agg_id = Uuid::new_v4().to_string();

    // Create workflow (starts in Planning phase)
    view.apply_event(
        &agg_id,
        &WorkflowEvent::WorkflowCreated {
            feature_name: FeatureName::from("test-feature"),
            objective: Objective::from("Test objective"),
            working_dir: WorkingDir::from(PathBuf::from("/tmp/workspace").as_path()),
            max_iterations: MaxIterations::default(),
            plan_path: PlanPath::from(PathBuf::from("/tmp/test-plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test-feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    view
}

#[test]
fn planning_system_prompt_references_skill() {
    assert!(
        PLANNING_SYSTEM_PROMPT.contains("planning"),
        "PLANNING_SYSTEM_PROMPT should reference the planning skill"
    );
    assert!(
        PLANNING_SYSTEM_PROMPT.contains("plan-output-path"),
        "PLANNING_SYSTEM_PROMPT should reference plan-output-path"
    );
}

#[test]
fn build_planning_prompt_includes_plan_output_path() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    assert!(
        prompt.contains("<plan-output-path>"),
        "Planning prompt should contain <plan-output-path> tag"
    );
    assert!(
        prompt.contains("/tmp/test-plan.md"),
        "Planning prompt should contain the plan file path"
    );
}

#[test]
fn build_planning_prompt_includes_session_folder() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    assert!(
        prompt.contains("<session-folder-path>"),
        "Planning prompt should contain <session-folder-path> tag"
    );
}

#[test]
fn build_planning_prompt_includes_workspace_root() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    assert!(
        prompt.contains("<workspace-root>"),
        "Planning prompt should contain <workspace-root> tag"
    );
    assert!(
        prompt.contains("/tmp/workspace"),
        "Planning prompt should contain the workspace path"
    );
}

#[test]
fn build_planning_prompt_includes_objective() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    assert!(
        prompt.contains("<objective>"),
        "Planning prompt should contain <objective> tag"
    );
    assert!(
        prompt.contains("Test objective"),
        "Planning prompt should contain the objective"
    );
}

#[test]
fn build_planning_prompt_references_skill() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    assert!(
        prompt.contains("planning"),
        "Planning prompt should reference the planning skill"
    );
}

#[test]
fn build_planning_prompt_includes_accumulated_user_feedback() {
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
            plan_path: PlanPath::from(PathBuf::from("/tmp/test-plan.md")),
            feedback_path: FeedbackPath::from(PathBuf::from("/tmp/test-feedback.md")),
            created_at: TimestampUtc::now(),
        },
        1,
    );

    // Simulate user declining twice with different feedback
    view.apply_event(&agg_id, &user_declined_event("Focus on error handling"), 2);
    view.apply_event(&agg_id, &user_declined_event("Add integration tests"), 3);

    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    // Prompt should contain the user-feedback tag and both feedback items
    assert!(
        prompt.contains("<user-feedback>"),
        "Planning prompt should contain <user-feedback> tag when feedback exists"
    );
    assert!(
        prompt.contains("Focus on error handling"),
        "Planning prompt should include first user feedback"
    );
    assert!(
        prompt.contains("Add integration tests"),
        "Planning prompt should include second user feedback"
    );
    // Verify numbering format
    assert!(
        prompt.contains("1. Focus on error handling"),
        "User feedback should be numbered"
    );
    assert!(
        prompt.contains("2. Add integration tests"),
        "User feedback should be numbered sequentially"
    );
}

#[test]
fn build_planning_prompt_excludes_user_feedback_when_empty() {
    let view = minimal_view();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&view, &working_dir);

    // Prompt should NOT contain user-feedback tag when there's no feedback
    assert!(
        !prompt.contains("<user-feedback>"),
        "Planning prompt should not contain <user-feedback> tag when no feedback exists"
    );
}
