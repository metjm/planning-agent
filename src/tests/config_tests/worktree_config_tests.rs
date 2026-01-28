//! Tests for WorktreeConfig parsing and defaults.

use super::*;

#[test]
fn test_worktree_config_enabled() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

worktree:
  enabled: true
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.worktree.enabled);
}

#[test]
fn test_worktree_config_disabled() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

worktree:
  enabled: false
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!config.worktree.enabled);
}

#[test]
fn test_worktree_config_missing_defaults_to_false() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    // Default should be false for backwards compatibility
    assert!(!config.worktree.enabled);
}

#[test]
fn test_worktree_config_empty_section_defaults() {
    let yaml = r#"
agents:
  claude:
    command: "claude"

workflow:
  planning:
    agent: claude
  reviewing:
    agents: [claude]

worktree: {}
"#;
    let config: WorkflowConfig = serde_yaml::from_str(yaml).unwrap();
    // Empty section should default to false
    assert!(!config.worktree.enabled);
}
