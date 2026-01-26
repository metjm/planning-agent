use super::*;
use crate::domain::types::{
    FeatureName, Iteration, MaxIterations, Objective, Phase, PlanPath, WorkflowId,
};
use crate::domain::view::WorkflowView;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

fn minimal_view() -> WorkflowView {
    WorkflowView {
        workflow_id: Some(WorkflowId(Uuid::new_v4())),
        feature_name: Some(FeatureName::from("test-feature")),
        objective: Some(Objective::from("Test objective")),
        working_dir: None,
        plan_path: Some(PlanPath::from(PathBuf::from("/tmp/test-plan/plan.md"))),
        feedback_path: None,
        planning_phase: Some(Phase::Complete),
        iteration: Some(Iteration::first()),
        max_iterations: Some(MaxIterations::default()),
        last_feedback_status: None,
        review_mode: None,
        implementation_state: None,
        agent_conversations: HashMap::new(),
        invocations: Vec::new(),
        last_failure: None,
        failure_history: Vec::new(),
        worktree_info: None,
        approval_overridden: false,
        last_event_sequence: 0,
    }
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
