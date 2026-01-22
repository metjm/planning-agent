//! Session snapshot conversion methods.
//!
//! This module provides conversion between Session and SessionUiState for
//! snapshot persistence.

use super::{FocusedPanel, InputMode, Session};
use crate::session_store::SessionUiState;
use crate::state::State;
use crate::tui::mention::MentionState;
use crate::tui::slash::SlashState;
use std::collections::HashMap;
use std::time::Instant;

impl Session {
    /// Converts session to a serializable UI state for snapshotting.
    pub fn to_ui_state(&self) -> SessionUiState {
        SessionUiState {
            id: self.id,
            name: self.name.clone(),
            status: self.status,
            output_lines: self.output_lines.clone(),
            scroll_position: self.scroll_position,
            output_follow_mode: self.output_follow_mode,
            streaming_lines: self.streaming_lines.clone(),
            streaming_scroll_position: self.streaming_scroll_position,
            streaming_follow_mode: self.streaming_follow_mode,
            focused_panel: self.focused_panel,
            total_cost: self.total_cost,
            bytes_received: self.bytes_received,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            total_cache_creation_tokens: self.total_cache_creation_tokens,
            total_cache_read_tokens: self.total_cache_read_tokens,
            tool_call_count: self.tool_call_count,
            bytes_per_second: self.bytes_per_second,
            turn_count: self.turn_count,
            model_name: self.model_name.clone(),
            last_stop_reason: self.last_stop_reason.clone(),
            tool_error_count: self.tool_error_count,
            total_tool_duration_ms: self.total_tool_duration_ms,
            completed_tool_count: self.completed_tool_count,
            approval_mode: self.approval_mode.clone(),
            approval_context: self.approval_context,
            plan_summary: self.plan_summary.clone(),
            plan_summary_scroll: self.plan_summary_scroll,
            user_feedback: self.user_feedback.clone(),
            cursor_position: self.cursor_position,
            feedback_scroll: self.feedback_scroll,
            feedback_target: self.feedback_target,
            input_mode: self.input_mode,
            tab_input: self.tab_input.clone(),
            tab_input_cursor: self.tab_input_cursor,
            tab_input_scroll: self.tab_input_scroll,
            last_key_was_backslash: self.last_key_was_backslash,
            tab_input_pastes: self.tab_input_pastes.clone(),
            feedback_pastes: self.feedback_pastes.clone(),
            error_state: self.error_state.clone(),
            error_scroll: self.error_scroll,
            run_tabs: self.run_tabs.clone(),
            active_run_tab: self.active_run_tab,
            chat_follow_mode: self.chat_follow_mode,
            todos: self.todos.clone(),
            todo_scroll_position: self.todo_scroll_position,
            account_usage: self.account_usage.clone(),
            spinner_frame: self.spinner_frame,
            current_run_id: self.current_run_id,
            plan_modal_open: self.plan_modal_open,
            plan_modal_scroll: self.plan_modal_scroll,
            review_history: self.review_history.clone(),
            review_history_spinner_frame: self.review_history_spinner_frame,
            review_history_scroll: self.review_history_scroll,
        }
    }

    /// Creates a session from a snapshot's UI state.
    /// Runtime fields (handles, channels, Instant) are initialized fresh.
    pub fn from_ui_state(ui_state: SessionUiState, workflow_state: Option<State>) -> Self {
        Self {
            id: ui_state.id,
            name: ui_state.name,
            status: ui_state.status,
            output_lines: ui_state.output_lines,
            scroll_position: ui_state.scroll_position,
            output_follow_mode: ui_state.output_follow_mode,
            streaming_lines: ui_state.streaming_lines,
            streaming_scroll_position: ui_state.streaming_scroll_position,
            streaming_follow_mode: ui_state.streaming_follow_mode,
            // Map Unknown variant (from old snapshots) to Output
            focused_panel: if ui_state.focused_panel == FocusedPanel::Unknown {
                FocusedPanel::Output
            } else {
                ui_state.focused_panel
            },
            workflow_state,
            state_snapshot: None,       // Will be populated when workflow spawns
            snapshot_rx: None,          // Will be populated when workflow spawns
            start_time: Instant::now(), // Reset to now
            total_cost: ui_state.total_cost,
            running: false,                        // Will be set when workflow resumes
            active_tools_by_agent: HashMap::new(), // Reset
            completed_tools_by_agent: HashMap::new(), // Reset
            cli_instances: Vec::new(),             // Runtime-only, reset on resume
            approval_mode: ui_state.approval_mode,
            approval_context: ui_state.approval_context,
            plan_summary: ui_state.plan_summary,
            plan_summary_scroll: ui_state.plan_summary_scroll,
            user_feedback: ui_state.user_feedback,
            cursor_position: ui_state.cursor_position,
            feedback_scroll: ui_state.feedback_scroll,
            // Map Unknown variant (from old snapshots) to Normal
            input_mode: if ui_state.input_mode == InputMode::Unknown {
                InputMode::Normal
            } else {
                ui_state.input_mode
            },
            tab_input: ui_state.tab_input,
            tab_input_cursor: ui_state.tab_input_cursor,
            tab_input_scroll: ui_state.tab_input_scroll,
            last_key_was_backslash: ui_state.last_key_was_backslash,
            tab_input_pastes: ui_state.tab_input_pastes,
            feedback_pastes: ui_state.feedback_pastes,
            error_state: ui_state.error_state,
            error_scroll: ui_state.error_scroll,
            bytes_received: ui_state.bytes_received,
            total_input_tokens: ui_state.total_input_tokens,
            total_output_tokens: ui_state.total_output_tokens,
            total_cache_creation_tokens: ui_state.total_cache_creation_tokens,
            total_cache_read_tokens: ui_state.total_cache_read_tokens,
            phase_times: HashMap::new(), // Reset
            current_phase_start: None,   // Reset
            tool_call_count: ui_state.tool_call_count,
            last_bytes_sample: (Instant::now(), 0), // Reset
            bytes_per_second: ui_state.bytes_per_second,
            turn_count: ui_state.turn_count,
            model_name: ui_state.model_name,
            last_stop_reason: ui_state.last_stop_reason,
            tool_error_count: ui_state.tool_error_count,
            total_tool_duration_ms: ui_state.total_tool_duration_ms,
            completed_tool_count: ui_state.completed_tool_count,
            workflow_handle: None,     // Reset
            approval_tx: None,         // Reset
            workflow_control_tx: None, // Reset
            feedback_target: ui_state.feedback_target,
            current_run_id: ui_state.current_run_id,
            account_usage: ui_state.account_usage,
            spinner_frame: ui_state.spinner_frame,
            run_tabs: ui_state.run_tabs,
            active_run_tab: ui_state.active_run_tab,
            chat_follow_mode: ui_state.chat_follow_mode,
            todos: ui_state.todos,
            todo_scroll_position: ui_state.todo_scroll_position,
            tab_mention_state: MentionState::new(), // Runtime-only, reset on resume
            feedback_mention_state: MentionState::new(), // Runtime-only, reset on resume
            tab_slash_state: SlashState::new(),     // Runtime-only, reset on resume
            plan_modal_open: ui_state.plan_modal_open,
            plan_modal_scroll: ui_state.plan_modal_scroll,
            plan_modal_content: String::new(), // Content is re-read from disk when modal opens

            review_history: ui_state.review_history,
            review_history_spinner_frame: ui_state.review_history_spinner_frame,
            review_history_scroll: ui_state.review_history_scroll,

            context: None, // Context is set by resume/new-session flows, not serialized

            implementation_success_modal: None, // Runtime-only, reset on restore
            implementation_interaction: super::ImplementationInteractionState {
                running: false,
                cancel_tx: None,
            },
        }
    }

    /// Adjusts the start_time to account for time elapsed in previous resume cycles.
    /// This makes `elapsed()` return the total time across all resume cycles.
    pub fn adjust_start_time_for_previous_elapsed(&mut self, previous_elapsed_ms: u64) {
        if previous_elapsed_ms > 0 {
            self.start_time -= std::time::Duration::from_millis(previous_elapsed_ms);
        }
    }
}
