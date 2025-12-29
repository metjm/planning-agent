
mod chat;
mod input;
pub mod model;
mod paste;

use crate::app::WorkflowResult;
use crate::cli_usage::AccountUsage;
use crate::state::{Phase, State};
use crate::tui::event::{TokenUsage, WorkflowCommand};
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
    /// Reserved for future ID-based tool correlation.
    #[allow(dead_code)]
    pub tool_id: Option<String>,
    /// Display name of the tool
    pub name: String,
    /// When the tool started
    pub started_at: Instant,
}


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

    /// Record that a tool has started for a specific agent
    pub fn tool_started(&mut self, name: String, agent_name: String) {
        let tool = ActiveTool {
            tool_id: None, // Will be set when we have ID-based correlation
            name,
            started_at: Instant::now(),
        };
        self.active_tools_by_agent
            .entry(agent_name)
            .or_default()
            .push(tool);
    }

    /// Remove the first tool for a specific agent (FIFO removal for ToolFinished events)
    pub fn tool_finished_for_agent(&mut self, agent_name: &str) {
        if let Some(tools) = self.active_tools_by_agent.get_mut(agent_name) {
            if !tools.is_empty() {
                tools.remove(0);
            }
            // Clean up empty agent entries
            if tools.is_empty() {
                self.active_tools_by_agent.remove(agent_name);
            }
        }
    }

    /// Remove a tool and return its duration (for ToolResult events)
    /// Returns Some(duration_ms) if a tool was found and removed, None otherwise
    pub fn tool_result_received_for_agent(&mut self, agent_name: &str) -> Option<u64> {
        if let Some(tools) = self.active_tools_by_agent.get_mut(agent_name) {
            if !tools.is_empty() {
                let tool = tools.remove(0);
                let duration_ms = tool.started_at.elapsed().as_millis() as u64;
                // Clean up empty agent entries
                if tools.is_empty() {
                    self.active_tools_by_agent.remove(agent_name);
                }
                return Some(duration_ms);
            }
        }
        None
    }

    /// Get all active tools across all agents as a flat list for compatibility
    pub fn all_active_tools(&self) -> Vec<(&str, &str, Instant)> {
        let mut tools = Vec::new();
        for (agent_name, agent_tools) in &self.active_tools_by_agent {
            for tool in agent_tools {
                tools.push((agent_name.as_str(), tool.name.as_str(), tool.started_at));
            }
        }
        tools
    }

    pub fn average_tool_duration_ms(&self) -> Option<u64> {
        if self.completed_tool_count > 0 {
            Some(self.total_tool_duration_ms / self.completed_tool_count as u64)
        } else {
            None
        }
    }

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
            None => "Initializing",
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
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests;
