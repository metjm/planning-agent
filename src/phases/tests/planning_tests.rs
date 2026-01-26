use super::*;
use crate::state::Phase;
use std::collections::HashMap;
use std::path::PathBuf;

fn minimal_state() -> State {
    State {
        phase: Phase::Planning,
        iteration: 1,
        max_iterations: 3,
        feature_name: "test-feature".to_string(),
        objective: "Test objective".to_string(),
        plan_file: PathBuf::from("/tmp/test-plan.md"),
        feedback_file: PathBuf::from("/tmp/test-feedback.md"),
        last_feedback_status: None,
        approval_overridden: false,
        workflow_session_id: "test-session".to_string(),
        agent_conversations: HashMap::new(),
        invocations: Vec::new(),
        updated_at: String::new(),
        last_failure: None,
        failure_history: Vec::new(),
        worktree_info: None,
        implementation_state: None,
        sequential_review: None,
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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&state, &working_dir);

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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&state, &working_dir);

    assert!(
        prompt.contains("<session-folder-path>"),
        "Planning prompt should contain <session-folder-path> tag"
    );
}

#[test]
fn build_planning_prompt_includes_workspace_root() {
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&state, &working_dir);

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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&state, &working_dir);

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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_planning_prompt(&state, &working_dir);

    assert!(
        prompt.contains("planning"),
        "Planning prompt should reference the planning skill"
    );
}
