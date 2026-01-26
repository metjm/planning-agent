//! Workflow browser overlay for viewing and selecting workflow configurations.
//!
//! This module provides a modal overlay that displays available workflows with
//! their agent configuration details, allowing users to:
//! - View built-in and custom workflows
//! - See planning and reviewing agents for each workflow
//! - Select a workflow for the current working directory
//! - Persist the selection across sessions

use crate::app::{list_available_workflows, load_workflow_by_name};
use crate::config::AggregationMode;
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
    /// Implementation agent name (or "disabled" if implementation not enabled)
    pub implementing_agent: String,
    /// Implementation review agent name (or "disabled" if implementation not enabled)
    pub implementation_reviewing_agent: String,
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

                        // Extract implementation phase agents
                        let (implementing, impl_reviewing) = if config.implementation.enabled {
                            (
                                config
                                    .implementation
                                    .implementing_agent()
                                    .unwrap_or("?")
                                    .to_string(),
                                config
                                    .implementation
                                    .reviewing_agent()
                                    .unwrap_or("?")
                                    .to_string(),
                            )
                        } else {
                            ("disabled".to_string(), "disabled".to_string())
                        };

                        WorkflowEntry {
                            name: wf.name,
                            source: wf.source,
                            is_selected: wf.is_selected,
                            planning_agent: planning,
                            reviewing_agents: reviewing.join(", "),
                            sequential_review: config.workflow.reviewing.sequential,
                            aggregation: agg.to_string(),
                            implementing_agent: implementing,
                            implementation_reviewing_agent: impl_reviewing,
                        }
                    }
                    Err(_) => WorkflowEntry {
                        name: wf.name,
                        source: wf.source,
                        is_selected: wf.is_selected,
                        planning_agent: "?".to_string(),
                        reviewing_agents: "?".to_string(),
                        sequential_review: false,
                        aggregation: "?".to_string(),
                        implementing_agent: "?".to_string(),
                        implementation_reviewing_agent: "?".to_string(),
                    },
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
#[path = "tests/workflow_browser_tests.rs"]
mod tests;
