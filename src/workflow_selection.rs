//! Workflow selection and persistence for the /workflow command.
//!
//! Provides per-working-directory workflow selection that persists across sessions.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Persisted workflow selection for a specific working directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSelection {
    /// Name of the selected workflow (e.g., "claude-only", "default", "my-workflow")
    pub workflow: String,
}

impl Default for WorkflowSelection {
    fn default() -> Self {
        Self {
            workflow: "claude-only".to_string(),
        }
    }
}

impl WorkflowSelection {
    /// Path to the workflow selection file for a working directory.
    /// Returns `~/.planning-agent/state/<wd-hash>/workflow-selection.json`
    pub fn selection_path(working_dir: &Path) -> Result<PathBuf> {
        Ok(crate::planning_paths::state_dir(working_dir)?.join("workflow-selection.json"))
    }

    /// Load the workflow selection for a working directory, or return default if not set.
    pub fn load(working_dir: &Path) -> Result<Self> {
        let path = Self::selection_path(working_dir)?;
        if path.exists() {
            let content = fs::read_to_string(&path).with_context(|| {
                format!("Failed to read workflow selection: {}", path.display())
            })?;
            serde_json::from_str(&content).with_context(|| "Failed to parse workflow selection")
        } else {
            Ok(Self::default())
        }
    }

    /// Save the workflow selection atomically using write-then-rename pattern.
    pub fn save(&self, working_dir: &Path) -> Result<()> {
        let path = Self::selection_path(working_dir)?;
        let temp_path = path.with_extension("json.tmp");

        // state_dir() already creates the directory, but ensure it exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize workflow selection")?;
        fs::write(&temp_path, &content)
            .with_context(|| format!("Failed to write temp file: {}", temp_path.display()))?;
        fs::rename(&temp_path, &path)
            .with_context(|| format!("Failed to rename to: {}", path.display()))?;
        Ok(())
    }
}

/// Information about an available workflow.
#[derive(Debug, Clone, Default)]
pub struct WorkflowInfo {
    /// Display name (e.g., "claude-only", "default", "my-custom")
    pub name: String,
    /// Source description (e.g., "built-in", "~/.planning-agent/workflows/my-custom.yaml")
    pub source: String,
}

/// Returns the workflows directory: `~/.planning-agent/workflows/`
pub fn workflows_dir() -> Result<PathBuf> {
    let dir = crate::planning_paths::planning_agent_home_dir()?.join("workflows");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create workflows directory: {}", dir.display()))?;
    Ok(dir)
}

/// Discover all available workflows for display in autocomplete.
/// Does not include selection state since that's per-working-directory.
pub fn list_available_workflows_for_display() -> Result<Vec<WorkflowInfo>> {
    // Built-in workflows
    let mut workflows = vec![
        WorkflowInfo {
            name: "default".to_string(),
            source: "built-in".to_string(),
        },
        WorkflowInfo {
            name: "claude-only".to_string(),
            source: "built-in".to_string(),
        },
        WorkflowInfo {
            name: "codex-only".to_string(),
            source: "built-in".to_string(),
        },
        WorkflowInfo {
            name: "gemini-only".to_string(),
            source: "built-in".to_string(),
        },
    ];

    // User workflows from ~/.planning-agent/workflows/
    let workflows_directory = workflows_dir()?;
    if workflows_directory.exists() {
        for entry in fs::read_dir(&workflows_directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                if let Some(stem) = path.file_stem() {
                    let name = stem.to_string_lossy().to_string();
                    // Skip if it conflicts with built-in names
                    if name != "default"
                        && name != "claude-only"
                        && name != "codex-only"
                        && name != "gemini-only"
                    {
                        workflows.push(WorkflowInfo {
                            name: name.clone(),
                            source: path.display().to_string(),
                        });
                    }
                }
            }
        }
    }

    // Sort alphabetically
    workflows.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(workflows)
}

/// Workflow info with selection state for a specific working directory.
#[derive(Debug, Clone)]
pub struct WorkflowInfoWithSelection {
    pub name: String,
    pub source: String,
    pub is_selected: bool,
}

/// Discover all available workflows with selection state for a specific working directory.
pub fn list_available_workflows(working_dir: &Path) -> Result<Vec<WorkflowInfoWithSelection>> {
    let current = WorkflowSelection::load(working_dir)?.workflow;
    let workflows = list_available_workflows_for_display()?;

    Ok(workflows
        .into_iter()
        .map(|wf| WorkflowInfoWithSelection {
            is_selected: wf.name == current,
            name: wf.name,
            source: wf.source,
        })
        .collect())
}

/// Load a workflow configuration by name.
pub fn load_workflow_by_name(name: &str) -> Result<crate::config::WorkflowConfig> {
    match name {
        "default" => Ok(crate::config::WorkflowConfig::default_config()),
        "claude-only" => Ok(crate::config::WorkflowConfig::claude_only_config()),
        "codex-only" => Ok(crate::config::WorkflowConfig::codex_only_config()),
        "gemini-only" => Ok(crate::config::WorkflowConfig::gemini_only_config()),
        _ => {
            let workflows_directory = workflows_dir()?;
            let yaml_path = workflows_directory.join(format!("{}.yaml", name));
            let yml_path = workflows_directory.join(format!("{}.yml", name));

            let path = if yaml_path.exists() {
                yaml_path
            } else if yml_path.exists() {
                yml_path
            } else {
                anyhow::bail!(
                    "Workflow '{}' not found in {}",
                    name,
                    workflows_directory.display()
                );
            };

            crate::config::WorkflowConfig::load(&path)
        }
    }
}

#[cfg(test)]
mod tests {
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
}
