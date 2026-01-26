use super::*;
use std::env;

fn test_working_dir() -> PathBuf {
    env::temp_dir().join(format!("workflow_selection_test_{}", std::process::id()))
}

#[test]
fn test_workflow_selection_default() {
    let selection = WorkflowSelection::default();
    assert_eq!(selection.workflow, "claude-only");
}

#[test]
fn test_workflow_selection_persistence() {
    let working_dir = test_working_dir();
    std::fs::create_dir_all(&working_dir).unwrap();

    // Initially should return default
    let initial = WorkflowSelection::load(&working_dir).unwrap();
    assert_eq!(initial.workflow, "claude-only");

    // Save a different selection
    let selection = WorkflowSelection {
        workflow: "default".to_string(),
    };
    selection.save(&working_dir).unwrap();

    // Load and verify
    let loaded = WorkflowSelection::load(&working_dir).unwrap();
    assert_eq!(loaded.workflow, "default");

    // Verify no temp file remains
    let temp_path = WorkflowSelection::selection_path(&working_dir)
        .unwrap()
        .with_extension("json.tmp");
    assert!(!temp_path.exists());

    // Cleanup
    std::fs::remove_dir_all(&working_dir).ok();
}

#[test]
fn test_per_working_directory_isolation() {
    let working_dir_a = test_working_dir().join("project_a");
    let working_dir_b = test_working_dir().join("project_b");
    std::fs::create_dir_all(&working_dir_a).unwrap();
    std::fs::create_dir_all(&working_dir_b).unwrap();

    // Save different selections for each directory
    WorkflowSelection {
        workflow: "default".to_string(),
    }
    .save(&working_dir_a)
    .unwrap();

    WorkflowSelection {
        workflow: "claude-only".to_string(),
    }
    .save(&working_dir_b)
    .unwrap();

    // Verify they are isolated
    let loaded_a = WorkflowSelection::load(&working_dir_a).unwrap();
    let loaded_b = WorkflowSelection::load(&working_dir_b).unwrap();

    assert_eq!(loaded_a.workflow, "default");
    assert_eq!(loaded_b.workflow, "claude-only");

    // Cleanup
    std::fs::remove_dir_all(test_working_dir()).ok();
}

#[test]
fn test_list_available_workflows_includes_builtins() {
    let workflows = list_available_workflows_for_display().unwrap();
    assert!(workflows.iter().any(|w| w.name == "default"));
    assert!(workflows.iter().any(|w| w.name == "claude-only"));
    assert!(workflows.iter().any(|w| w.name == "codex-only"));
    assert!(workflows.iter().any(|w| w.name == "gemini-only"));
}

#[test]
fn test_load_builtin_workflows() {
    // Test loading built-in workflows
    let default = load_workflow_by_name("default").unwrap();
    assert!(!default.agents.is_empty());

    let claude_only = load_workflow_by_name("claude-only").unwrap();
    assert!(!claude_only.agents.is_empty());

    let codex_only = load_workflow_by_name("codex-only").unwrap();
    assert!(!codex_only.agents.is_empty());

    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    assert!(!gemini_only.agents.is_empty());
}

#[test]
fn test_load_nonexistent_workflow() {
    let result = load_workflow_by_name("nonexistent-workflow-xyz");
    assert!(result.is_err());
}

#[test]
fn test_codex_only_workflow_uses_codex_for_planning() {
    let codex_only = load_workflow_by_name("codex-only").unwrap();
    assert_eq!(codex_only.workflow.planning.agent, "codex");
}

#[test]
fn test_codex_only_workflow_has_implementation_enabled() {
    let codex_only = load_workflow_by_name("codex-only").unwrap();
    assert!(codex_only.implementation.enabled);
}

#[test]
fn test_codex_only_workflow_has_codex_reviewer() {
    let codex_only = load_workflow_by_name("codex-only").unwrap();
    assert!(codex_only.agents.contains_key("codex-reviewer"));
}

#[test]
fn test_codex_only_workflow_uses_correct_impl_agents() {
    let codex_only = load_workflow_by_name("codex-only").unwrap();
    assert_eq!(
        codex_only.implementation.implementing_agent(),
        Some("codex")
    );
    assert_eq!(
        codex_only.implementation.reviewing_agent(),
        Some("codex-reviewer")
    );
}

#[test]
fn test_codex_reviewer_has_same_command_as_codex() {
    let codex_only = load_workflow_by_name("codex-only").unwrap();
    let codex = codex_only.agents.get("codex").unwrap();
    let codex_reviewer = codex_only.agents.get("codex-reviewer").unwrap();
    assert_eq!(codex.command, codex_reviewer.command);
}

#[test]
fn test_gemini_only_workflow_uses_gemini_for_planning() {
    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    assert_eq!(gemini_only.workflow.planning.agent, "gemini");
}

#[test]
fn test_gemini_only_workflow_has_implementation_enabled() {
    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    assert!(gemini_only.implementation.enabled);
}

#[test]
fn test_gemini_only_workflow_has_gemini_reviewer() {
    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    assert!(gemini_only.agents.contains_key("gemini-reviewer"));
}

#[test]
fn test_gemini_only_workflow_uses_correct_impl_agents() {
    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    assert_eq!(
        gemini_only.implementation.implementing_agent(),
        Some("gemini")
    );
    assert_eq!(
        gemini_only.implementation.reviewing_agent(),
        Some("gemini-reviewer")
    );
}

#[test]
fn test_gemini_reviewer_has_same_command_as_gemini() {
    let gemini_only = load_workflow_by_name("gemini-only").unwrap();
    let gemini = gemini_only.agents.get("gemini").unwrap();
    let gemini_reviewer = gemini_only.agents.get("gemini-reviewer").unwrap();
    assert_eq!(gemini.command, gemini_reviewer.command);
}
