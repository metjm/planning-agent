mod approval;
mod chat;
mod cli_instances;
pub mod context;
mod input;
mod modals;
pub mod model;
mod paste;
mod snapshot;
mod tools;

pub use cli_instances::{CliInstance, CliInstanceId};

use crate::app::AccountUsage;
use crate::app::WorkflowResult;
use crate::domain::view::WorkflowView;
use crate::phases::implementing_conversation_key;
use crate::state::{ImplementationPhase, Phase, State, UiMode};
use crate::tui::event::{TokenUsage, WorkflowCommand};
use crate::tui::mention::MentionState;
use crate::tui::slash::SlashState;
use anyhow::Result;
pub use context::SessionContext;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::event::UserApprovalResponse;

pub use model::{
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, ImplementationSuccessModal,
    InputMode, PasteBlock, ReviewKind, ReviewModalEntry, ReviewRound, ReviewerEntry,
    ReviewerStatus, RunTab, RunTabEntry, SessionStatus, SummaryState, TodoItem, TodoStatus,
    ToolKind, ToolResultSummary, ToolTimelineEntry,
};

/// Represents an active tool call with optional ID for correlation
#[derive(Debug, Clone)]
pub struct ActiveTool {
    /// Optional unique identifier for correlating with ToolResult.
    pub tool_id: Option<String>,
    /// Human-readable display name of the tool
    pub display_name: String,
    /// Compact preview of tool input (e.g., file path, command)
    pub input_preview: String,
    /// When the tool started
    pub started_at: Instant,
}

/// Represents a completed tool call
#[derive(Debug, Clone)]
pub struct CompletedTool {
    /// When the tool completed (for ordering/truncation)
    pub completed_at: Instant,
}

/// Summary info returned on tool completion for timeline updates.
#[derive(Debug, Clone)]
pub struct ToolCompletionInfo {
    pub display_name: String,
    pub input_preview: String,
    pub duration_ms: u64,
    pub is_error: bool,
}

/// Maximum number of completed tools to retain per session
const MAX_COMPLETED_TOOLS: usize = 100;

pub struct Session {
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

    /// Full workflow state (legacy, for persistence).
    pub workflow_state: Option<State>,
    /// Event-sourced workflow view (replaces workflow_state in CQRS mode).
    pub workflow_view: Option<WorkflowView>,
    pub start_time: Instant,
    pub total_cost: f64,
    pub running: bool,
    /// Active tools grouped by agent name
    pub active_tools_by_agent: HashMap<String, Vec<ActiveTool>>,
    /// Completed tools grouped by agent name (newest first)
    pub completed_tools_by_agent: HashMap<String, Vec<CompletedTool>>,
    /// Active CLI agent instances (runtime-only, not serialized)
    pub cli_instances: Vec<CliInstance>,

    pub approval_mode: ApprovalMode,
    pub approval_context: ApprovalContext,
    pub plan_summary: String,
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,
    pub feedback_scroll: usize,

    pub input_mode: InputMode,
    pub tab_input: String,
    pub tab_input_cursor: usize,
    pub tab_input_scroll: usize,

    pub last_key_was_backslash: bool,

    pub tab_input_pastes: Vec<PasteBlock>,

    pub feedback_pastes: Vec<PasteBlock>,

    pub error_state: Option<String>,
    pub error_scroll: usize,

    pub bytes_received: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub phase_times: HashMap<String, Duration>,
    pub current_phase_start: Option<(String, Instant)>,
    pub tool_call_count: usize,
    pub last_bytes_sample: (Instant, usize),
    pub bytes_per_second: f64,
    pub turn_count: u32,
    pub model_name: Option<String>,
    pub last_stop_reason: Option<String>,
    pub tool_error_count: usize,
    pub total_tool_duration_ms: u64,
    pub completed_tool_count: usize,

    pub workflow_handle: Option<JoinHandle<Result<WorkflowResult>>>,
    pub approval_tx: Option<mpsc::Sender<UserApprovalResponse>>,
    /// Channel to send commands (like interrupt) to the running workflow.
    pub workflow_control_tx: Option<mpsc::Sender<WorkflowCommand>>,
    /// Tracks the target of the current feedback entry mode.
    pub feedback_target: FeedbackTarget,
    /// Tracks the current run ID for scoping summary events.
    pub current_run_id: u64,

    pub account_usage: AccountUsage,

    pub spinner_frame: u8,

    pub run_tabs: Vec<RunTab>,
    pub active_run_tab: usize,
    pub chat_follow_mode: bool,

    /// Review history: rounds and their reviewer statuses
    pub review_history: Vec<ReviewRound>,
    /// Spinner frame for review history panel
    pub review_history_spinner_frame: u8,
    /// Scroll position for review history panel
    pub review_history_scroll: usize,

    pub todos: HashMap<String, Vec<TodoItem>>,
    pub todo_scroll_position: usize,

    /// @-mention state for tab input field
    pub tab_mention_state: MentionState,
    /// @-mention state for feedback input field
    pub feedback_mention_state: MentionState,
    /// Slash command autocomplete state for tab input field
    pub tab_slash_state: SlashState,

    /// Whether the plan modal is currently open
    pub plan_modal_open: bool,
    /// Scroll position within the plan modal
    pub plan_modal_scroll: usize,
    /// Cached plan modal content (runtime-only, not serialized)
    pub plan_modal_content: String,

    /// Whether the review modal is currently open
    pub review_modal_open: bool,
    /// Scroll position within the review modal content
    pub review_modal_scroll: usize,
    /// Currently selected review tab index (0 = most recent)
    pub review_modal_tab: usize,
    /// Loaded review entries: (display_name, file_path, content, sort_key)
    /// Sorted by (iteration DESC, agent_name ASC) for deterministic ordering.
    pub review_modal_entries: Vec<ReviewModalEntry>,

    /// Per-session context tracking working directory, paths, and configuration.
    /// None for sessions created before this feature or not yet initialized.
    pub context: Option<SessionContext>,

    /// Runtime-only modal for implementation success display.
    /// Not serialized - always None on snapshot restore.
    pub implementation_success_modal: Option<ImplementationSuccessModal>,
    /// Runtime-only state for post-implementation interaction.
    /// Not serialized - always reset on snapshot restore.
    pub implementation_interaction: ImplementationInteractionState,
}

/// Runtime-only state for post-implementation interaction.
#[derive(Debug)]
pub struct ImplementationInteractionState {
    pub running: bool,
    pub cancel_tx: Option<watch::Sender<bool>>,
}

/// Session provides the full API surface for session management.
/// Some methods may not be used in all code paths but are part of the public API.
impl Session {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            name: String::new(),
            status: SessionStatus::InputPending,

            output_lines: Vec::new(),
            scroll_position: 0,
            output_follow_mode: true,
            streaming_lines: Vec::new(),
            streaming_scroll_position: 0,
            streaming_follow_mode: true,
            focused_panel: FocusedPanel::default(),

            workflow_state: None,
            workflow_view: None, // Set when CQRS workflow sends view updates
            start_time: Instant::now(),
            total_cost: 0.0,
            running: true,
            active_tools_by_agent: HashMap::new(),
            completed_tools_by_agent: HashMap::new(),
            cli_instances: Vec::new(),

            approval_mode: ApprovalMode::None,
            approval_context: ApprovalContext::PlanApproval,
            plan_summary: String::new(),
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,
            feedback_scroll: 0,

            input_mode: InputMode::Normal,
            tab_input: String::new(),
            tab_input_cursor: 0,
            tab_input_scroll: 0,
            last_key_was_backslash: false,

            tab_input_pastes: Vec::new(),
            feedback_pastes: Vec::new(),

            error_state: None,
            error_scroll: 0,

            bytes_received: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            phase_times: HashMap::new(),
            current_phase_start: None,
            tool_call_count: 0,
            last_bytes_sample: (Instant::now(), 0),
            bytes_per_second: 0.0,
            turn_count: 0,
            model_name: None,
            last_stop_reason: None,
            tool_error_count: 0,
            total_tool_duration_ms: 0,
            completed_tool_count: 0,

            workflow_handle: None,
            approval_tx: None,
            workflow_control_tx: None,
            feedback_target: FeedbackTarget::default(),
            current_run_id: 0,

            account_usage: AccountUsage::default(),

            spinner_frame: 0,

            run_tabs: Vec::new(),
            active_run_tab: 0,
            chat_follow_mode: true,

            review_history: Vec::new(),
            review_history_spinner_frame: 0,
            review_history_scroll: 0,

            todos: HashMap::new(),
            todo_scroll_position: 0,

            tab_mention_state: MentionState::new(),
            feedback_mention_state: MentionState::new(),
            tab_slash_state: SlashState::new(),

            plan_modal_open: false,
            plan_modal_scroll: 0,
            plan_modal_content: String::new(),

            review_modal_open: false,
            review_modal_scroll: 0,
            review_modal_tab: 0,
            review_modal_entries: Vec::new(),

            context: None,

            implementation_success_modal: None,
            implementation_interaction: ImplementationInteractionState {
                running: false,
                cancel_tx: None,
            },
        }
    }

    pub fn with_name(id: usize, name: String) -> Self {
        let mut session = Self::new(id);
        session.name = name;
        session.input_mode = InputMode::Normal;
        session.status = SessionStatus::Planning;
        session
    }

    pub fn handle_error(&mut self, error: &str) {
        self.error_state = Some(error.to_string());
        self.error_scroll = 0;
        self.workflow_handle = None;
        self.workflow_control_tx = None;
        self.status = SessionStatus::Error;
    }

    pub fn clear_error(&mut self) {
        self.error_state = None;
        self.error_scroll = 0;
    }

    /// Returns the feature name from workflow view, workflow state, or session name.
    pub fn feature_name(&self) -> &str {
        self.workflow_view
            .as_ref()
            .and_then(|v| v.feature_name.as_ref())
            .map(|f| f.as_str())
            .or_else(|| {
                self.workflow_state
                    .as_ref()
                    .map(|s| s.feature_name.as_str())
            })
            .unwrap_or(&self.name)
    }

    pub fn error_scroll_up(&mut self) {
        self.error_scroll = self.error_scroll.saturating_sub(1);
    }

    pub fn error_scroll_down(&mut self, max_scroll: usize) {
        if self.error_scroll < max_scroll {
            self.error_scroll += 1;
        }
    }

    pub fn add_bytes(&mut self, bytes: usize) {
        self.bytes_received += bytes;
        self.update_bytes_rate();
    }

    fn update_bytes_rate(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_bytes_sample.0);
        if elapsed.as_millis() >= 500 {
            let bytes_delta = self.bytes_received.saturating_sub(self.last_bytes_sample.1);
            self.bytes_per_second = bytes_delta as f64 / elapsed.as_secs_f64();
            self.last_bytes_sample = (now, self.bytes_received);
        }
    }

    pub fn add_token_usage(&mut self, usage: &TokenUsage) {
        self.total_input_tokens += usage.input_tokens;
        self.total_output_tokens += usage.output_tokens;
        self.total_cache_creation_tokens += usage.cache_creation_tokens;
        self.total_cache_read_tokens += usage.cache_read_tokens;
    }

    pub fn start_phase(&mut self, phase: String) {
        if let Some((prev_phase, start)) = self.current_phase_start.take() {
            let duration = start.elapsed();
            *self.phase_times.entry(prev_phase).or_default() += duration;
        }
        self.current_phase_start = Some((phase, Instant::now()));
    }

    // Note: Approval methods (start_approval, start_review_decision, etc.) are in approval.rs
    // Note: Tool tracking methods (tool_started, tool_finished_for_agent, etc.) are in tools.rs

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn add_output(&mut self, line: String) {
        self.output_lines.push(line);

        self.output_follow_mode = true;
        self.scroll_position = self.output_lines.len().saturating_sub(1);
    }

    pub fn scroll_up(&mut self) {
        self.output_follow_mode = false;
        self.scroll_position = self.scroll_position.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, max_scroll: usize) {
        if self.scroll_position < max_scroll {
            self.scroll_position = self.scroll_position.saturating_add(1);
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.output_follow_mode = false;
        self.scroll_position = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.output_follow_mode = true;
        self.scroll_position = self.output_lines.len().saturating_sub(1);
    }

    pub fn add_streaming(&mut self, line: String) {
        self.streaming_lines.push(line);

        self.streaming_follow_mode = true;
        self.streaming_scroll_position = self.streaming_lines.len().saturating_sub(1);
    }

    /// Toggle focus between panels, considering visibility of Todos panel.
    /// `todos_visible` indicates whether the Todos panel is currently visible
    /// (based on terminal width and whether todos exist).
    pub fn toggle_focus_with_visibility(&mut self, todos_visible: bool) {
        let has_summary = self
            .run_tabs
            .get(self.active_run_tab)
            .map(|tab| tab.summary_state != SummaryState::None)
            .unwrap_or(false);
        let can_interact = self.can_interact_with_implementation();

        self.focused_panel = match self.focused_panel {
            FocusedPanel::Output => {
                if todos_visible {
                    FocusedPanel::Todos
                } else {
                    FocusedPanel::Chat
                }
            }
            FocusedPanel::Todos => FocusedPanel::Chat,
            FocusedPanel::Chat => {
                if can_interact {
                    FocusedPanel::ChatInput
                } else if has_summary {
                    FocusedPanel::Summary
                } else {
                    FocusedPanel::Output
                }
            }
            FocusedPanel::ChatInput => {
                if has_summary {
                    FocusedPanel::Summary
                } else {
                    FocusedPanel::Output
                }
            }
            FocusedPanel::Summary => FocusedPanel::Output,
        };
    }

    /// Check if the current focused panel is Todos but todos are not visible.
    /// Used to reset focus when the panel becomes invisible due to resize or clearing.
    pub fn is_focus_on_invisible_todos(&self, todos_visible: bool) -> bool {
        self.focused_panel == FocusedPanel::Todos && !todos_visible
    }

    /// Reset focus from Todos to Output if todos are not visible.
    pub fn reset_focus_if_todos_invisible(&mut self, todos_visible: bool) {
        if self.is_focus_on_invisible_todos(todos_visible) {
            self.focused_panel = FocusedPanel::Output;
        }
    }

    /// Returns true if implementation follow-up interaction is available.
    pub fn can_interact_with_implementation(&self) -> bool {
        let Some(state) = self.workflow_state.as_ref() else {
            return false;
        };
        let Some(impl_state) = state.implementation_state.as_ref() else {
            return false;
        };
        if impl_state.phase != ImplementationPhase::Complete {
            return false;
        }
        let Some(context) = self.context.as_ref() else {
            return false;
        };
        if !context.workflow_config.implementation.enabled {
            return false;
        }
        let Some(agent_name) = context.workflow_config.implementation.implementing_agent() else {
            return false;
        };
        let conversation_key = implementing_conversation_key(agent_name);
        state
            .agent_conversations
            .get(&conversation_key)
            .and_then(|conv| conv.conversation_id.as_ref())
            .is_some()
    }

    /// Returns the current UI mode (Planning or Implementation).
    /// Used by the theme system to determine which color palette to use.
    pub fn ui_mode(&self) -> UiMode {
        match &self.workflow_state {
            Some(state) => state.workflow_stage(),
            None => UiMode::Planning,
        }
    }

    pub fn phase_name(&self) -> &str {
        match &self.workflow_state {
            Some(state) => {
                // If implementation is active, show implementation sub-phase
                if let Some(impl_state) = &state.implementation_state {
                    if impl_state.phase != ImplementationPhase::Complete {
                        return impl_state.phase.label();
                    }
                }
                // Otherwise show planning workflow phase
                match state.phase {
                    Phase::Planning => "Planning",
                    Phase::Reviewing => "Reviewing",
                    Phase::Revising => "Revising",
                    Phase::AwaitingPlanningDecision => "Awaiting Decision",
                    Phase::Complete => "Complete",
                }
            }
            None => "Initializing",
        }
    }

    pub fn iteration(&self) -> (u32, u32) {
        match &self.workflow_state {
            Some(state) => {
                // If implementation is active, show implementation iteration
                if let Some(impl_state) = &state.implementation_state {
                    if impl_state.phase != ImplementationPhase::Complete {
                        return (impl_state.iteration, impl_state.max_iterations);
                    }
                }
                // Otherwise show planning workflow iteration
                (state.iteration, state.max_iterations)
            }
            None => (0, 0),
        }
    }

    pub fn update_todos(&mut self, agent_name: String, todos: Vec<TodoItem>) {
        self.todos.insert(agent_name, todos);
    }

    pub fn clear_todos(&mut self) {
        self.todos.clear();
        self.todo_scroll_position = 0;
    }

    pub fn todo_scroll_up(&mut self) {
        self.todo_scroll_position = self.todo_scroll_position.saturating_sub(1);
    }

    pub fn todo_scroll_down(&mut self, max_scroll: usize) {
        if self.todo_scroll_position < max_scroll {
            self.todo_scroll_position += 1;
        }
    }

    pub fn todo_scroll_to_top(&mut self) {
        self.todo_scroll_position = 0;
    }

    pub fn todo_scroll_to_bottom(&mut self, max_scroll: usize) {
        self.todo_scroll_position = max_scroll;
    }

    pub fn get_todos_display(&self) -> Vec<String> {
        let mut lines = Vec::new();

        let mut agent_names: Vec<_> = self.todos.keys().collect();
        agent_names.sort();

        for agent_name in agent_names {
            if let Some(todos) = self.todos.get(agent_name) {
                if todos.is_empty() {
                    continue;
                }

                let display_name = agent_name
                    .chars()
                    .next()
                    .map(|c| {
                        let rest = agent_name
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| agent_name.get(i..).unwrap_or(""))
                            .unwrap_or("");
                        c.to_uppercase().to_string() + rest
                    })
                    .unwrap_or_else(|| agent_name.clone());
                lines.push(format!("{}:", display_name));
                for todo in todos {
                    let status = match todo.status {
                        TodoStatus::Pending => "[ ]",
                        TodoStatus::InProgress => "[~]",
                        TodoStatus::Completed => "[x]",
                    };
                    lines.push(format!("  {} {}", status, todo.active_form));
                }
                lines.push(String::new());
            }
        }

        if lines.is_empty() {
            lines.push("No todos".to_string());
        }

        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        }

        lines
    }

    pub fn display_cost(&self) -> f64 {
        self.total_cost
    }

    // Review history methods are implemented in review_history.rs

    // Note: `to_ui_state` and `from_ui_state` are implemented in snapshot.rs

    /// Accept the currently selected mention in the tab input field.
    /// Replaces the @query with the selected file path (absolute path when available).
    pub fn accept_tab_mention(&mut self) {
        if let Some(selected) = self.tab_mention_state.selected_match() {
            let path = selected.insert_text();
            let start = self.tab_mention_state.start_byte;
            let end = self.tab_input_cursor;

            // Replace @query with the file path
            let before = self.tab_input.get(..start).unwrap_or("");
            let after = self.tab_input.get(end..).unwrap_or("");
            self.tab_input = format!("{}{} {}", before, path, after);

            // Move cursor to after the inserted path + space
            self.tab_input_cursor = start + path.len() + 1;

            // Clear mention state
            self.tab_mention_state.clear();
        }
    }

    /// Accept the currently selected mention in the feedback input field.
    /// Replaces the @query with the selected file path (absolute path when available).
    pub fn accept_feedback_mention(&mut self) {
        if let Some(selected) = self.feedback_mention_state.selected_match() {
            let path = selected.insert_text();
            let start = self.feedback_mention_state.start_byte;
            let end = self.cursor_position;

            // Replace @query with the file path
            let before = self.user_feedback.get(..start).unwrap_or("");
            let after = self.user_feedback.get(end..).unwrap_or("");
            self.user_feedback = format!("{}{} {}", before, path, after);

            // Move cursor to after the inserted path + space
            self.cursor_position = start + path.len() + 1;

            // Clear mention state
            self.feedback_mention_state.clear();
        }
    }

    /// Accept the currently selected slash command in the tab input field.
    /// Replaces the command token with the selected command.
    pub fn accept_tab_slash(&mut self) {
        if let Some(selected) = self.tab_slash_state.selected_match() {
            let insert = selected.insert.clone();
            let start = self.tab_slash_state.start_byte;
            let end = self.tab_slash_state.end_byte;

            // Replace the command token with the selected command
            let before = self.tab_input.get(..start).unwrap_or("");
            let after = self.tab_input.get(end..).unwrap_or("");
            self.tab_input = format!("{}{}{}", before, insert, after);

            // Move cursor to the end of the inserted command
            self.tab_input_cursor = start + insert.len();

            // Clear slash state
            self.tab_slash_state.clear();
        }
    }

    // Note: Plan modal methods (toggle_plan_modal, plan_modal_scroll_*, etc.) are in plan_modal.rs
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tools_tests.rs"]
mod tests_tools;
