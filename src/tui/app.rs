use crate::state::{Phase, State};
use std::time::{Duration, Instant};

/// Mode for user approval interaction
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalMode {
    /// Normal workflow mode
    None,
    /// Showing summary, waiting for accept/decline choice
    AwaitingChoice,
    /// User chose decline, now entering feedback
    EnteringFeedback,
}

pub struct App {
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub streaming_lines: Vec<String>,
    pub streaming_scroll_position: usize,
    pub workflow_state: Option<State>,
    pub start_time: Instant,
    pub total_cost: f64,
    pub running: bool,
    pub should_quit: bool,
    pub active_tools: Vec<(String, Instant)>, // (tool_name, start_time)

    // User approval state
    pub approval_mode: ApprovalMode,
    pub plan_summary: String,
    pub user_feedback: String,
    pub cursor_position: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            output_lines: Vec::new(),
            scroll_position: 0,
            streaming_lines: Vec::new(),
            streaming_scroll_position: 0,
            workflow_state: None,
            start_time: Instant::now(),
            total_cost: 0.0,
            running: true,
            should_quit: false,
            active_tools: Vec::new(),
            approval_mode: ApprovalMode::None,
            plan_summary: String::new(),
            user_feedback: String::new(),
            cursor_position: 0,
        }
    }

    pub fn start_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.user_feedback.clear();
        self.cursor_position = 0;
    }

    pub fn start_feedback_input(&mut self) {
        self.approval_mode = ApprovalMode::EnteringFeedback;
        self.user_feedback.clear();
        self.cursor_position = 0;
    }

    pub fn insert_char(&mut self, c: char) {
        self.user_feedback.insert(self.cursor_position, c);
        self.cursor_position += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
            self.user_feedback.remove(self.cursor_position);
        }
    }

    pub fn move_cursor_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.user_feedback.len() {
            self.cursor_position += 1;
        }
    }

    pub fn tool_started(&mut self, name: String) {
        self.active_tools.push((name, Instant::now()));
    }

    pub fn tool_finished(&mut self, name: &str) {
        self.active_tools.retain(|(n, _)| n != name);
    }

    pub fn clear_active_tools(&mut self) {
        self.active_tools.clear();
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn add_output(&mut self, line: String) {
        self.output_lines.push(line);
        // Auto-scroll to bottom when new output arrives
        self.scroll_to_bottom();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_position = self.scroll_position.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self.output_lines.len().saturating_sub(1);
        self.scroll_position = (self.scroll_position + 1).min(max_scroll);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_position = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_position = self.output_lines.len().saturating_sub(1);
    }

    pub fn add_streaming(&mut self, line: String) {
        self.streaming_lines.push(line);
        // Auto-scroll to bottom when new streaming content arrives
        self.streaming_scroll_to_bottom();
    }

    pub fn streaming_scroll_to_bottom(&mut self) {
        self.streaming_scroll_position = self.streaming_lines.len().saturating_sub(1);
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
        match &self.workflow_state {
            Some(state) => &state.feature_name,
            None => "...",
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
