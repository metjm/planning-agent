use super::*;
use std::path::PathBuf;

fn minimal_state() -> State {
    use crate::state::Phase;
    use std::collections::HashMap;

    State {
        phase: Phase::Complete,
        iteration: 1,
        max_iterations: 3,
        feature_name: "test-feature".to_string(),
        objective: "Test objective".to_string(),
        plan_file: PathBuf::from("/tmp/test-plan/plan.md"),
        feedback_file: PathBuf::from("/tmp/test-feedback.md"),
        last_feedback_status: None,
        approval_overridden: false,
        workflow_session_id: "test-session-id".to_string(),
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
fn test_build_implementation_prompt_basic() {
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_prompt(&state, &working_dir, 1, None);

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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let feedback = "Missing error handling in src/main.rs";
    let prompt = build_implementation_prompt(&state, &working_dir, 2, Some(feedback));

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
    let state = minimal_state();
    let working_dir = PathBuf::from("/tmp/workspace");
    let prompt = build_implementation_followup_prompt(&state, &working_dir, "Fix the bug");

    assert!(prompt.contains("USER MESSAGE"));
    assert!(prompt.contains("Fix the bug"));
    assert!(prompt.contains("/tmp/workspace"));
}
