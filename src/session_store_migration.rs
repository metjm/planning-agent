//! Migration support for session snapshot versions.
//!
//! This module handles conversion of legacy snapshot formats to the current version.

use crate::cli_usage::{AccountUsage, ProviderUsage};
use crate::session_store::{SessionSnapshot, SessionUiState, SNAPSHOT_VERSION};
use crate::tui::session::model::{
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, PasteBlock, RunTab,
    SessionStatus, TodoItem,
};
use crate::state::State;
use crate::usage_reset::UsageWindow;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// v1 -> v2 migration: session_used/weekly_used -> UsageWindow
// ============================================================================

/// Legacy v1 ProviderUsage with session_used/weekly_used fields
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyProviderUsageV1 {
    pub provider: String,
    pub display_name: String,
    pub session_used: Option<u8>,
    pub weekly_used: Option<u8>,
    pub plan_type: Option<String>,
    pub status_message: Option<String>,
    pub supports_usage: bool,
}

/// Legacy v1 AccountUsage
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyAccountUsageV1 {
    pub providers: HashMap<String, LegacyProviderUsageV1>,
}

/// Legacy v1 SessionUiState with old AccountUsage format
#[derive(Debug, Clone, Deserialize)]
pub struct LegacySessionUiStateV1 {
    pub id: usize,
    pub name: String,
    pub status: SessionStatus,
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub output_follow_mode: bool,
    pub streaming_lines: Vec<String>,
    pub streaming_scroll_position: usize,
    pub streaming_follow_mode: bool,
    pub focused_panel: FocusedPanel,
    pub total_cost: f64,
    pub bytes_received: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub tool_call_count: usize,
    pub bytes_per_second: f64,
    pub turn_count: u32,
    pub model_name: Option<String>,
    pub last_stop_reason: Option<String>,
    pub tool_error_count: usize,
    pub total_tool_duration_ms: u64,
    pub completed_tool_count: usize,
    pub approval_mode: ApprovalMode,
    pub approval_context: ApprovalContext,
    pub plan_summary: String,
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,
    pub feedback_scroll: usize,
    pub feedback_target: FeedbackTarget,
    pub input_mode: InputMode,
    pub tab_input: String,
    pub tab_input_cursor: usize,
    pub tab_input_scroll: usize,
    pub last_key_was_backslash: bool,
    pub tab_input_pastes: Vec<PasteBlock>,
    pub feedback_pastes: Vec<PasteBlock>,
    pub error_state: Option<String>,
    pub error_scroll: usize,
    pub run_tabs: Vec<RunTab>,
    pub active_run_tab: usize,
    pub chat_follow_mode: bool,
    pub todos: HashMap<String, Vec<TodoItem>>,
    pub todo_scroll_position: usize,
    pub account_usage: LegacyAccountUsageV1,
    pub spinner_frame: u8,
    pub current_run_id: u64,
    #[serde(default)]
    pub plan_modal_open: bool,
    #[serde(default)]
    pub plan_modal_scroll: usize,
}

/// Legacy v1 SessionSnapshot
#[derive(Debug, Clone, Deserialize)]
pub struct LegacySessionSnapshotV1 {
    #[allow(dead_code)]
    pub version: u32,
    pub saved_at: String,
    pub working_dir: PathBuf,
    pub workflow_session_id: String,
    pub state_path: PathBuf,
    pub workflow_state: State,
    pub ui_state: LegacySessionUiStateV1,
    pub total_elapsed_before_resume_ms: u64,
}

/// Migrate a v1 snapshot to v2 format
pub fn migrate_v1_to_v2(legacy: LegacySessionSnapshotV1) -> SessionSnapshot {
    // Convert legacy ProviderUsage to new format
    let mut new_providers = HashMap::new();
    for (key, old) in legacy.ui_state.account_usage.providers {
        let session = old
            .session_used
            .map(UsageWindow::with_percent)
            .unwrap_or_default();
        let weekly = old
            .weekly_used
            .map(UsageWindow::with_percent)
            .unwrap_or_default();

        new_providers.insert(
            key,
            ProviderUsage {
                provider: old.provider,
                display_name: old.display_name,
                session,
                weekly,
                plan_type: old.plan_type,
                fetched_at: None, // Cannot restore Instant across sessions
                status_message: old.status_message,
                supports_usage: old.supports_usage,
            },
        );
    }

    let new_account_usage = AccountUsage {
        providers: new_providers,
    };

    // Build new UI state
    let new_ui_state = SessionUiState {
        id: legacy.ui_state.id,
        name: legacy.ui_state.name,
        status: legacy.ui_state.status,
        output_lines: legacy.ui_state.output_lines,
        scroll_position: legacy.ui_state.scroll_position,
        output_follow_mode: legacy.ui_state.output_follow_mode,
        streaming_lines: legacy.ui_state.streaming_lines,
        streaming_scroll_position: legacy.ui_state.streaming_scroll_position,
        streaming_follow_mode: legacy.ui_state.streaming_follow_mode,
        focused_panel: legacy.ui_state.focused_panel,
        total_cost: legacy.ui_state.total_cost,
        bytes_received: legacy.ui_state.bytes_received,
        total_input_tokens: legacy.ui_state.total_input_tokens,
        total_output_tokens: legacy.ui_state.total_output_tokens,
        total_cache_creation_tokens: legacy.ui_state.total_cache_creation_tokens,
        total_cache_read_tokens: legacy.ui_state.total_cache_read_tokens,
        tool_call_count: legacy.ui_state.tool_call_count,
        bytes_per_second: legacy.ui_state.bytes_per_second,
        turn_count: legacy.ui_state.turn_count,
        model_name: legacy.ui_state.model_name,
        last_stop_reason: legacy.ui_state.last_stop_reason,
        tool_error_count: legacy.ui_state.tool_error_count,
        total_tool_duration_ms: legacy.ui_state.total_tool_duration_ms,
        completed_tool_count: legacy.ui_state.completed_tool_count,
        approval_mode: legacy.ui_state.approval_mode,
        approval_context: legacy.ui_state.approval_context,
        plan_summary: legacy.ui_state.plan_summary,
        plan_summary_scroll: legacy.ui_state.plan_summary_scroll,
        user_feedback: legacy.ui_state.user_feedback,
        cursor_position: legacy.ui_state.cursor_position,
        feedback_scroll: legacy.ui_state.feedback_scroll,
        feedback_target: legacy.ui_state.feedback_target,
        input_mode: legacy.ui_state.input_mode,
        tab_input: legacy.ui_state.tab_input,
        tab_input_cursor: legacy.ui_state.tab_input_cursor,
        tab_input_scroll: legacy.ui_state.tab_input_scroll,
        last_key_was_backslash: legacy.ui_state.last_key_was_backslash,
        tab_input_pastes: legacy.ui_state.tab_input_pastes,
        feedback_pastes: legacy.ui_state.feedback_pastes,
        error_state: legacy.ui_state.error_state,
        error_scroll: legacy.ui_state.error_scroll,
        run_tabs: legacy.ui_state.run_tabs,
        active_run_tab: legacy.ui_state.active_run_tab,
        chat_follow_mode: legacy.ui_state.chat_follow_mode,
        todos: legacy.ui_state.todos,
        todo_scroll_position: legacy.ui_state.todo_scroll_position,
        account_usage: new_account_usage,
        spinner_frame: legacy.ui_state.spinner_frame,
        current_run_id: legacy.ui_state.current_run_id,
        plan_modal_open: legacy.ui_state.plan_modal_open,
        plan_modal_scroll: legacy.ui_state.plan_modal_scroll,
    };

    SessionSnapshot {
        version: SNAPSHOT_VERSION,
        saved_at: legacy.saved_at,
        working_dir: legacy.working_dir,
        workflow_session_id: legacy.workflow_session_id,
        state_path: legacy.state_path,
        workflow_state: legacy.workflow_state,
        ui_state: new_ui_state,
        total_elapsed_before_resume_ms: legacy.total_elapsed_before_resume_ms,
    }
}
