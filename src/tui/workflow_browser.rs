//! Workflow browser overlay for viewing and selecting workflow configurations.
//!
//! This module provides a modal overlay that displays available workflows with
//! their agent configuration details, allowing users to:
//! - View built-in and custom workflows
//! - See planning and reviewing agents for each workflow
//! - Select a workflow for the current working directory
//! - Persist the selection across sessions

use crate::config::AggregationMode;
use crate::workflow_selection::{list_available_workflows, load_workflow_by_name};
use std::path::{Path, PathBuf};

/// A workflow entry with display information.
#[derive(Debug, Clone)]
pub struct WorkflowEntry {
    /// Workflow name (e.g., "default", "claude-only", "my-workflow")
    pub name: String,
    /// Source description (e.g., "built-in", "~/.planning-agent/workflows/...")
    pub source: String,
    /// Whether this workflow is currently selected for the working directory
    pub is_selected: bool,
    /// Planning agent name
    pub planning_agent: String,
    /// Reviewing agent names (comma-separated)
    pub reviewing_agents: String,
    /// Whether sequential review is enabled
    pub sequential_review: bool,
    /// Aggregation mode description
    pub aggregation: String,
}

/// State for the workflow browser overlay.
#[derive(Debug, Clone)]
pub struct WorkflowBrowserState {
    /// Whether the overlay is open
    pub open: bool,
    /// List of available workflows
    pub entries: Vec<WorkflowEntry>,
    /// Currently selected index
    pub selected_idx: usize,
    /// Scroll offset for the list
    pub scroll_offset: usize,
    /// Current working directory (for persistence)
    pub working_dir: PathBuf,
}

impl Default for WorkflowBrowserState {
    fn default() -> Self {
        Self {
            open: false,
            entries: Vec::new(),
            selected_idx: 0,
            scroll_offset: 0,
            working_dir: PathBuf::new(),
        }
    }
}

impl WorkflowBrowserState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the browser and loads workflows.
    pub fn open(&mut self, working_dir: &Path) {
        self.open = true;
        self.selected_idx = 0;
        self.scroll_offset = 0;
        self.working_dir = working_dir.to_path_buf();
        self.refresh(working_dir);
    }

    /// Closes the browser overlay.
    pub fn close(&mut self) {
        self.open = false;
        self.entries.clear();
    }

    /// Refreshes the workflow list from disk.
    pub fn refresh(&mut self, working_dir: &Path) {
        let workflows = list_available_workflows(working_dir).unwrap_or_default();

        self.entries = workflows
            .into_iter()
            .map(|wf| {
                // Load config to extract agent details
                let (planning, reviewing, sequential, aggregation) =
                    match load_workflow_by_name(&wf.name) {
                        Ok(config) => {
                            let planning = config.workflow.planning.agent.clone();
                            let reviewing: Vec<_> = config
                                .workflow
                                .reviewing
                                .agents
                                .iter()
                                .map(|a| a.display_id().to_string())
                                .collect();
                            let agg = match config.workflow.reviewing.aggregation {
                                AggregationMode::AnyRejects => "any-rejects",
                                AggregationMode::AllReject => "all-reject",
                                AggregationMode::Majority => "majority",
                            };
                            (
                                planning,
                                reviewing.join(", "),
                                config.workflow.reviewing.sequential,
                                agg.to_string(),
                            )
                        }
                        Err(_) => ("?".to_string(), "?".to_string(), false, "?".to_string()),
                    };

                WorkflowEntry {
                    name: wf.name,
                    source: wf.source,
                    is_selected: wf.is_selected,
                    planning_agent: planning,
                    reviewing_agents: reviewing,
                    sequential_review: sequential,
                    aggregation,
                }
            })
            .collect();

        // Pre-select the currently selected workflow
        if let Some(idx) = self.entries.iter().position(|e| e.is_selected) {
            self.selected_idx = idx;
            self.ensure_visible();
        }
    }

    /// Moves selection up with wrapping.
    pub fn select_prev(&mut self) {
        if !self.entries.is_empty() {
            if self.selected_idx == 0 {
                self.selected_idx = self.entries.len() - 1;
            } else {
                self.selected_idx -= 1;
            }
            self.ensure_visible();
        }
    }

    /// Moves selection down with wrapping.
    pub fn select_next(&mut self) {
        if !self.entries.is_empty() {
            self.selected_idx = (self.selected_idx + 1) % self.entries.len();
            self.ensure_visible();
        }
    }

    /// Returns the currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&WorkflowEntry> {
        self.entries.get(self.selected_idx)
    }

    /// Ensure the selected item is visible in the viewport.
    fn ensure_visible(&mut self) {
        const VIEWPORT_SIZE: usize = 8;

        if self.selected_idx < self.scroll_offset {
            self.scroll_offset = self.selected_idx;
        } else if self.selected_idx >= self.scroll_offset + VIEWPORT_SIZE {
            self.scroll_offset = self.selected_idx.saturating_sub(VIEWPORT_SIZE - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_browser_state_new() {
        let state = WorkflowBrowserState::new();
        assert!(!state.open);
        assert!(state.entries.is_empty());
        assert_eq!(state.selected_idx, 0);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_workflow_browser_close() {
        let mut state = WorkflowBrowserState::new();
        state.open = true;
        state.entries.push(WorkflowEntry {
            name: "test".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
        });

        state.close();
        assert!(!state.open);
        assert!(state.entries.is_empty());
    }

    #[test]
    fn test_select_prev_wraps() {
        let mut state = WorkflowBrowserState::new();
        state.entries = vec![
            WorkflowEntry {
                name: "a".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
            WorkflowEntry {
                name: "b".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
            WorkflowEntry {
                name: "c".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
        ];
        state.selected_idx = 0;

        state.select_prev();
        assert_eq!(state.selected_idx, 2); // Should wrap to end
    }

    #[test]
    fn test_select_next_wraps() {
        let mut state = WorkflowBrowserState::new();
        state.entries = vec![
            WorkflowEntry {
                name: "a".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
            WorkflowEntry {
                name: "b".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
            WorkflowEntry {
                name: "c".to_string(),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            },
        ];
        state.selected_idx = 2;

        state.select_next();
        assert_eq!(state.selected_idx, 0); // Should wrap to start
    }

    #[test]
    fn test_selected_entry() {
        let mut state = WorkflowBrowserState::new();
        assert!(state.selected_entry().is_none());

        state.entries.push(WorkflowEntry {
            name: "test".to_string(),
            source: "built-in".to_string(),
            is_selected: false,
            planning_agent: "claude".to_string(),
            reviewing_agents: "claude".to_string(),
            sequential_review: false,
            aggregation: "any-rejects".to_string(),
        });
        state.selected_idx = 0;

        let entry = state.selected_entry().unwrap();
        assert_eq!(entry.name, "test");
    }

    #[test]
    fn test_ensure_visible_scrolls_up() {
        let mut state = WorkflowBrowserState::new();
        // Add 15 entries
        for i in 0..15 {
            state.entries.push(WorkflowEntry {
                name: format!("workflow-{}", i),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            });
        }

        // Start scrolled down
        state.scroll_offset = 10;
        state.selected_idx = 5;
        state.ensure_visible();

        // Should scroll up to show selected
        assert!(state.scroll_offset <= state.selected_idx);
    }

    #[test]
    fn test_ensure_visible_scrolls_down() {
        let mut state = WorkflowBrowserState::new();
        // Add 15 entries
        for i in 0..15 {
            state.entries.push(WorkflowEntry {
                name: format!("workflow-{}", i),
                source: "built-in".to_string(),
                is_selected: false,
                planning_agent: "claude".to_string(),
                reviewing_agents: "claude".to_string(),
                sequential_review: false,
                aggregation: "any-rejects".to_string(),
            });
        }

        // Start at top
        state.scroll_offset = 0;
        state.selected_idx = 12;
        state.ensure_visible();

        // Should scroll down to show selected (viewport size is 8)
        assert!(state.selected_idx < state.scroll_offset + 8);
    }

    #[test]
    fn test_refresh_loads_builtin_workflows() {
        let mut state = WorkflowBrowserState::new();
        let temp_dir = std::env::temp_dir();

        state.refresh(&temp_dir);

        // Should have at least the built-in workflows
        assert!(state.entries.iter().any(|e| e.name == "default"));
        assert!(state.entries.iter().any(|e| e.name == "claude-only"));
    }

    #[test]
    fn test_refresh_preselects_current_workflow() {
        let mut state = WorkflowBrowserState::new();
        let temp_dir = std::env::temp_dir();

        // Simulate a workflow being marked as selected
        state.refresh(&temp_dir);

        // The default selection is "claude-only", so it should be pre-selected
        if let Some(idx) = state.entries.iter().position(|e| e.is_selected) {
            assert_eq!(state.selected_idx, idx);
        }
    }
}
