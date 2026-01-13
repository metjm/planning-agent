
mod chat;
mod input;
pub mod model;
mod paste;
mod snapshot;
mod tools;

use crate::app::WorkflowResult;
use crate::cli_usage::AccountUsage;
use crate::state::{Phase, State};
use crate::tui::embedded_terminal::EmbeddedTerminal;
use crate::tui::event::{TokenUsage, WorkflowCommand};
use crate::tui::mention::MentionState;
use crate::tui::slash::SlashState;
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::event::UserApprovalResponse;

pub use model::{
    ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, PasteBlock, RunTab,
    SessionStatus, SummaryState, TodoItem, TodoStatus,
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
    /// Human-readable display name of the tool
    pub display_name: String,
    /// Compact preview of tool input (e.g., file path, command)
    pub input_preview: String,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Whether the tool execution resulted in an error
    pub is_error: bool,
    /// When the tool completed (for ordering/truncation)
    pub completed_at: Instant,
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

    pub workflow_state: Option<State>,
    pub start_time: Instant,
    pub total_cost: f64,
    pub running: bool,
    /// Active tools grouped by agent name
    pub active_tools_by_agent: HashMap<String, Vec<ActiveTool>>,
    /// Completed tools grouped by agent name (newest first)
    pub completed_tools_by_agent: HashMap<String, Vec<CompletedTool>>,

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

    pub todos: HashMap<String, Vec<TodoItem>>,
    pub todo_scroll_position: usize,

    /// Embedded implementation terminal (runtime-only, not serialized)
    pub implementation_terminal: Option<EmbeddedTerminal>,

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
}

/// Session provides the full API surface for session management.
/// Some methods may not be used in all code paths but are part of the public API.
#[allow(dead_code)]
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
            start_time: Instant::now(),
            total_cost: 0.0,
            running: true,
            active_tools_by_agent: HashMap::new(),
            completed_tools_by_agent: HashMap::new(),

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

            todos: HashMap::new(),
            todo_scroll_position: 0,

            implementation_terminal: None,

            tab_mention_state: MentionState::new(),
            feedback_mention_state: MentionState::new(),
            tab_slash_state: SlashState::new(),

            plan_modal_open: false,
            plan_modal_scroll: 0,
            plan_modal_content: String::new(),
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

    pub fn start_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::PlanApproval;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_review_decision(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::ReviewDecision;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_max_iterations_prompt(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::MaxIterationsReached;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_plan_generation_failed(&mut self, error: String) {
        self.plan_summary = error;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::PlanGenerationFailed;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_user_override_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::UserOverrideApproval;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn start_all_reviewers_failed(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.approval_context = ApprovalContext::AllReviewersFailed;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.status = SessionStatus::AwaitingApproval;
    }

    pub fn scroll_summary_up(&mut self) {
        self.plan_summary_scroll = self.plan_summary_scroll.saturating_sub(1);
    }

    pub fn scroll_summary_down(&mut self, max_scroll: usize) {
        if self.plan_summary_scroll < max_scroll {
            self.plan_summary_scroll += 1;
        }
    }

    pub fn start_feedback_input(&mut self) {
        self.start_feedback_input_for(FeedbackTarget::ApprovalDecline);
    }

    pub fn start_feedback_input_for(&mut self, target: FeedbackTarget) {
        self.approval_mode = ApprovalMode::EnteringFeedback;
        self.feedback_target = target;
        self.user_feedback.clear();
        self.cursor_position = 0;
        self.feedback_scroll = 0;
    }

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

    pub fn scroll_down(&mut self) {

        self.scroll_position = self.scroll_position.saturating_add(1);
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

    pub fn streaming_scroll_up(&mut self) {

        self.streaming_follow_mode = false;
        self.streaming_scroll_position = self.streaming_scroll_position.saturating_sub(1);
    }

    pub fn streaming_scroll_down(&mut self) {

        self.streaming_scroll_position = self.streaming_scroll_position.saturating_add(1);
    }

    pub fn streaming_scroll_to_bottom(&mut self) {

        self.streaming_follow_mode = true;
        self.streaming_scroll_position = self.streaming_lines.len().saturating_sub(1);
    }

    pub fn toggle_focus(&mut self) {
        self.toggle_focus_with_visibility(false)
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
                if has_summary {
                    FocusedPanel::Summary
                } else {
                    FocusedPanel::Output
                }
            }
            FocusedPanel::Summary => FocusedPanel::Output,
            FocusedPanel::Implementation => FocusedPanel::Output,
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

    pub fn phase_name(&self) -> &str {
        match &self.workflow_state {
            Some(state) => match state.phase {
                Phase::Planning => "Planning",
                Phase::Reviewing => "Reviewing",
                Phase::Revising => "Revising",
                Phase::Complete => "Complete",
            },
            None => match self.status {
                SessionStatus::Verifying => "Verifying",
                SessionStatus::Fixing => "Fixing",
                SessionStatus::VerificationComplete => "Verified",
                _ => "Initializing",
            },
        }
    }

    /// Starts the verification phase
    pub fn start_verification(&mut self, iteration: u32) {
        self.status = SessionStatus::Verifying;
        self.add_output(format!(
            "[verification] Starting verification round {}",
            iteration
        ));
    }

    /// Handles verification completion with a verdict
    pub fn handle_verification_completed(&mut self, verdict: &str, report: &str) {
        self.add_output(format!("[verification] Verdict: {}", verdict));
        if !report.is_empty() {
            // Show a preview of the report
            let preview: String = report.lines().take(5).collect::<Vec<_>>().join("\n");
            self.add_output(format!("[verification] Report preview:\n{}", preview));
        }
    }

    /// Starts the fixing phase
    pub fn start_fixing(&mut self, iteration: u32) {
        self.status = SessionStatus::Fixing;
        self.add_output(format!("[fixing] Starting fix round {}", iteration));
    }

    /// Handles fixing completion
    pub fn handle_fixing_completed(&mut self) {
        self.add_output("[fixing] Fix round complete".to_string());
    }

    /// Handles verification workflow result
    pub fn handle_verification_result(&mut self, approved: bool, iterations_used: u32) {
        if approved {
            self.status = SessionStatus::VerificationComplete;
            self.running = false;
            self.add_output(format!(
                "[verification] Implementation APPROVED after {} iteration(s)",
                iterations_used
            ));
        } else {
            self.status = SessionStatus::Error;
            self.running = false;
            self.error_state = Some(format!(
                "Verification FAILED after {} iteration(s)",
                iterations_used
            ));
        }
    }

    pub fn iteration(&self) -> (u32, u32) {
        match &self.workflow_state {
            Some(state) => (state.iteration, state.max_iterations),
            None => (0, 0),
        }
    }

    pub fn feature_name(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else {
            match &self.workflow_state {
                Some(state) => &state.feature_name,
                None => "New Tab",
            }
        }
    }

    pub fn handle_completion(&mut self, result: WorkflowResult) {
        match result {
            WorkflowResult::Accepted => {
                self.status = SessionStatus::Complete;
                self.running = false;
            }
            WorkflowResult::NeedsRestart { .. } => {
                self.status = SessionStatus::Planning;
            }
            WorkflowResult::Aborted { reason } => {
                self.status = SessionStatus::Error;
                self.running = false;
                self.error_state = Some(reason);
            }
            WorkflowResult::Stopped => {
                self.status = SessionStatus::Stopped;
                self.running = false;
                // Note: snapshot saving happens in the caller (tui_runner/events.rs)
                // before handle_completion is called
            }
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
                    .map(|c| c.to_uppercase().to_string() + &agent_name[1..])
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

    // Note: `to_ui_state` and `from_ui_state` are implemented in snapshot.rs

    /// Check if implementation terminal is active
    pub fn has_active_implementation_terminal(&self) -> bool {
        self.implementation_terminal
            .as_ref()
            .map(|t| t.active)
            .unwrap_or(false)
    }

    /// Stop the implementation terminal and return to normal mode
    pub fn stop_implementation_terminal(&mut self) {
        if let Some(mut terminal) = self.implementation_terminal.take() {
            terminal.kill();
        }
        self.input_mode = InputMode::Normal;
        self.focused_panel = FocusedPanel::Output;
    }

    /// Accept the currently selected mention in the tab input field.
    /// Replaces the @query with the selected file path.
    pub fn accept_tab_mention(&mut self) {
        if let Some(selected) = self.tab_mention_state.selected_match() {
            let path = selected.path.clone();
            let start = self.tab_mention_state.start_byte;
            let end = self.tab_input_cursor;

            // Replace @query with the file path
            let before = &self.tab_input[..start];
            let after = &self.tab_input[end..];
            self.tab_input = format!("{}{} {}", before, path, after);

            // Move cursor to after the inserted path + space
            self.tab_input_cursor = start + path.len() + 1;

            // Clear mention state
            self.tab_mention_state.clear();
        }
    }

    /// Accept the currently selected mention in the feedback input field.
    /// Replaces the @query with the selected file path.
    pub fn accept_feedback_mention(&mut self) {
        if let Some(selected) = self.feedback_mention_state.selected_match() {
            let path = selected.path.clone();
            let start = self.feedback_mention_state.start_byte;
            let end = self.cursor_position;

            // Replace @query with the file path
            let before = &self.user_feedback[..start];
            let after = &self.user_feedback[end..];
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
            let before = &self.tab_input[..start];
            let after = &self.tab_input[end..];
            self.tab_input = format!("{}{}{}", before, insert, after);

            // Move cursor to the end of the inserted command
            self.tab_input_cursor = start + insert.len();

            // Clear slash state
            self.tab_slash_state.clear();
        }
    }

    /// Toggle the plan modal open/closed.
    /// When opening, reads the plan file from disk and populates plan_modal_content.
    /// Returns true if the modal was opened, false if it was closed or no plan file exists.
    pub fn toggle_plan_modal(&mut self, working_dir: &std::path::Path) -> bool {
        if self.plan_modal_open {
            // Close the modal
            self.plan_modal_open = false;
            self.plan_modal_content.clear();
            false
        } else {
            // Try to open the modal
            if let Some(ref state) = self.workflow_state {
                let plan_path = if state.plan_file.is_absolute() {
                    state.plan_file.clone()
                } else {
                    working_dir.join(&state.plan_file)
                };

                match std::fs::read_to_string(&plan_path) {
                    Ok(content) => {
                        self.plan_modal_content = content;
                        self.plan_modal_open = true;
                        self.plan_modal_scroll = 0;
                        true
                    }
                    Err(e) => {
                        self.plan_modal_content =
                            format!("Unable to read plan file:\n{}\n\nError: {}", plan_path.display(), e);
                        self.plan_modal_open = true;
                        self.plan_modal_scroll = 0;
                        true
                    }
                }
            } else {
                // No workflow state, cannot open modal
                false
            }
        }
    }

    /// Close the plan modal if it's open.
    pub fn close_plan_modal(&mut self) {
        self.plan_modal_open = false;
        self.plan_modal_content.clear();
    }

    /// Scroll the plan modal up by one line.
    pub fn plan_modal_scroll_up(&mut self) {
        self.plan_modal_scroll = self.plan_modal_scroll.saturating_sub(1);
    }

    /// Scroll the plan modal down by one line, respecting max_scroll.
    pub fn plan_modal_scroll_down(&mut self, max_scroll: usize) {
        if self.plan_modal_scroll < max_scroll {
            self.plan_modal_scroll += 1;
        }
    }

    /// Scroll the plan modal to the top.
    pub fn plan_modal_scroll_to_top(&mut self) {
        self.plan_modal_scroll = 0;
    }

    /// Scroll the plan modal to the bottom.
    pub fn plan_modal_scroll_to_bottom(&mut self, max_scroll: usize) {
        self.plan_modal_scroll = max_scroll;
    }

    /// Scroll the plan modal by a page (visible height).
    pub fn plan_modal_page_down(&mut self, visible_height: usize, max_scroll: usize) {
        self.plan_modal_scroll = (self.plan_modal_scroll + visible_height).min(max_scroll);
    }

    /// Scroll the plan modal up by a page (visible height).
    pub fn plan_modal_page_up(&mut self, visible_height: usize) {
        self.plan_modal_scroll = self.plan_modal_scroll.saturating_sub(visible_height);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_tools;
