use super::*;
use crate::domain::types::{FeatureName, Iteration, MaxIterations, Objective, Phase, PlanPath};
use crate::domain::view::WorkflowView;
use std::path::PathBuf;
use uuid::Uuid;

fn minimal_view() -> WorkflowView {
    WorkflowView {
        workflow_id: Some(crate::domain::types::WorkflowId(Uuid::new_v4())),
        feature_name: Some(FeatureName::from("test-feature")),
        objective: Some(Objective::from("Test objective")),
        working_dir: None,
        plan_path: Some(PlanPath::from(PathBuf::from("/tmp/test-plan.md"))),
        feedback_path: None,
        planning_phase: Some(Phase::Planning),
        iteration: Some(Iteration::first()),
        max_iterations: Some(MaxIterations::default()),
        last_feedback_status: None,
        review_mode: None,
        implementation_state: None,
        agent_conversations: std::collections::HashMap::new(),
        invocations: Vec::new(),
        last_failure: None,
        failure_history: Vec::new(),
        worktree_info: None,
        approval_overridden: false,
        last_event_sequence: 0,
    }
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
