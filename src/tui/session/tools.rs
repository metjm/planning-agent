//! Tool tracking methods for Session.
//!
//! This module provides tool lifecycle tracking (started, finished, result received)
//! for the TUI session.

use super::{ActiveTool, CompletedTool, Session, ToolCompletionInfo, MAX_COMPLETED_TOOLS};
use std::time::Instant;

impl Session {
    /// Record that a tool has started for a specific agent
    pub fn tool_started(
        &mut self,
        tool_id: Option<String>,
        display_name: String,
        input_preview: String,
        agent_name: String,
    ) {
        let tool = ActiveTool {
            tool_id,
            display_name,
            input_preview,
            started_at: Instant::now(),
        };
        self.active_tools_by_agent
            .entry(agent_name)
            .or_default()
            .push(tool);
    }

    /// Handle ToolFinished events - this is a no-op if the tool was already
    /// completed by ToolResultReceived (prevents double-removal).
    /// Uses ID-based matching with FIFO fallback.
    pub fn tool_finished_for_agent(&mut self, tool_id: Option<&str>, agent_name: &str) {
        if let Some(tools) = self.active_tools_by_agent.get_mut(agent_name) {
            // Normalize empty string to None
            let normalized_id = tool_id.filter(|s| !s.is_empty());

            // Try ID-based matching first if ID is provided
            let found_idx = if let Some(id) = normalized_id {
                tools.iter().position(|t| t.tool_id.as_deref() == Some(id))
            } else {
                None
            };

            // If ID match failed (or no ID), fall back to FIFO
            // This handles the Gemini case where starts have None but results have function name
            if found_idx.is_none() && !tools.is_empty() {
                tools.remove(0);
            } else if let Some(idx) = found_idx {
                tools.remove(idx);
            }

            // Clean up empty agent entries
            if tools.is_empty() {
                self.active_tools_by_agent.remove(agent_name);
            }
        }
    }

    /// Remove a tool and return completion info (for ToolResult events).
    /// Uses ID-based matching with FIFO fallback.
    pub fn tool_result_received_for_agent(
        &mut self,
        tool_id: Option<&str>,
        is_error: bool,
        agent_name: &str,
    ) -> ToolCompletionInfo {
        // Normalize empty string to None
        let normalized_id = tool_id.filter(|s| !s.is_empty());

        // First, check if we have active tools for this agent and find the matching tool
        let tool_info: Option<(String, String, u64)> = {
            if let Some(tools) = self.active_tools_by_agent.get_mut(agent_name) {
                if tools.is_empty() {
                    None
                } else {
                    // Try ID-based matching first if ID is provided
                    let found_idx = if let Some(id) = normalized_id {
                        tools.iter().position(|t| t.tool_id.as_deref() == Some(id))
                    } else {
                        None
                    };

                    // If ID match failed (or no ID), fall back to FIFO (index 0)
                    // This handles the Gemini case where starts have None but results have function name
                    let idx = found_idx.unwrap_or(0);

                    let tool = tools.remove(idx);
                    let duration_ms = tool.started_at.elapsed().as_millis() as u64;
                    Some((tool.display_name, tool.input_preview, duration_ms))
                }
            } else {
                None
            }
        };

        let (display_name, input_preview, duration_ms) =
            if let Some((display_name, input_preview, duration_ms)) = tool_info {
                let completed_at = Instant::now();

                // Move to completed tools list
                let completed_tool = CompletedTool {
                    display_name: display_name.clone(),
                    input_preview: input_preview.clone(),
                    duration_ms,
                    is_error,
                    completed_at,
                };

                // Insert at the beginning for reverse chronological order (newest first)
                self.completed_tools_by_agent
                    .entry(agent_name.to_string())
                    .or_default()
                    .insert(0, completed_tool);

                // Enforce retention cap
                self.trim_completed_tools();

                // Clean up empty active tools entries
                if let Some(tools) = self.active_tools_by_agent.get(agent_name) {
                    if tools.is_empty() {
                        self.active_tools_by_agent.remove(agent_name);
                    }
                }
                (display_name, input_preview, duration_ms)
            } else {
                // No active tools found - create a synthetic completed entry only for true orphan results
                // (This is an edge case where we got a result without a matching start)
                let display_name = normalized_id.unwrap_or("unknown").to_string();
                let input_preview = String::new();
                let duration_ms = 0;

                let completed_tool = CompletedTool {
                    display_name: display_name.clone(),
                    input_preview: input_preview.clone(),
                    duration_ms,
                    is_error,
                    completed_at: Instant::now(),
                };

                self.completed_tools_by_agent
                    .entry(agent_name.to_string())
                    .or_default()
                    .insert(0, completed_tool);

                self.trim_completed_tools();

                (display_name, input_preview, duration_ms)
            };

        ToolCompletionInfo {
            display_name,
            input_preview,
            duration_ms,
            is_error,
        }
    }

    /// Trim completed tools to stay under the retention cap
    pub fn trim_completed_tools(&mut self) {
        // Count total completed tools
        let total: usize = self
            .completed_tools_by_agent
            .values()
            .map(|v| v.len())
            .sum();

        if total <= MAX_COMPLETED_TOOLS {
            return;
        }

        // Need to drop (total - MAX_COMPLETED_TOOLS) oldest entries
        let to_drop = total - MAX_COMPLETED_TOOLS;

        // Collect all completed tools with their agent names to find oldest
        let mut all_tools: Vec<(String, Instant)> = Vec::new();
        for (agent, tools) in &self.completed_tools_by_agent {
            for tool in tools {
                all_tools.push((agent.clone(), tool.completed_at));
            }
        }

        // Sort by completed_at (oldest first)
        all_tools.sort_by_key(|(_, t)| *t);

        // Find the cutoff time
        if let Some((_, cutoff_time)) = all_tools.get(to_drop.saturating_sub(1)) {
            let cutoff = *cutoff_time;

            // Remove tools older than or equal to cutoff
            for tools in self.completed_tools_by_agent.values_mut() {
                let mut dropped = 0;
                tools.retain(|t| {
                    if dropped < to_drop && t.completed_at <= cutoff {
                        dropped += 1;
                        false
                    } else {
                        true
                    }
                });
            }

            // Clean up empty agent entries
            self.completed_tools_by_agent.retain(|_, v| !v.is_empty());
        }
    }

    /// Get all completed tools across all agents as a flat list
    /// Note: Currently unused after replacing draw_tool_calls_panel with
    /// draw_reviewer_history_panel, but kept for potential future tool tracking features.
    #[allow(dead_code)]
    pub fn all_completed_tools(&self) -> Vec<(&str, &CompletedTool)> {
        let mut tools = Vec::new();
        for (agent_name, agent_tools) in &self.completed_tools_by_agent {
            for tool in agent_tools {
                tools.push((agent_name.as_str(), tool));
            }
        }
        // Sort by completed_at descending (newest first)
        tools.sort_by(|a, b| b.1.completed_at.cmp(&a.1.completed_at));
        tools
    }

    pub fn average_tool_duration_ms(&self) -> Option<u64> {
        if self.completed_tool_count > 0 {
            Some(self.total_tool_duration_ms / self.completed_tool_count as u64)
        } else {
            None
        }
    }
}
