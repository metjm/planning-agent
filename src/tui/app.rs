use crate::state::{Phase, State};
use crate::tui::event::TokenUsage;
use std::collections::HashMap;
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
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,

    // Stats
    pub bytes_received: usize,

    // Token stats
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub total_cache_read_tokens: u64,

    // Phase timing
    pub phase_times: HashMap<String, Duration>,
    pub current_phase_start: Option<(String, Instant)>,

    // Tool stats
    pub tool_call_count: usize,
    pub tool_output_lines: HashMap<String, usize>,

    // Streaming rate
    pub last_bytes_sample: (Instant, usize),
    pub bytes_per_second: f64,
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
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,
            bytes_received: 0,
            // Token stats
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_creation_tokens: 0,
            total_cache_read_tokens: 0,
            // Phase timing
            phase_times: HashMap::new(),
            current_phase_start: None,
            // Tool stats
            tool_call_count: 0,
            tool_output_lines: HashMap::new(),
            // Streaming rate
            last_bytes_sample: (Instant::now(), 0),
            bytes_per_second: 0.0,
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
        // End previous phase if any
        if let Some((prev_phase, start)) = self.current_phase_start.take() {
            let duration = start.elapsed();
            *self.phase_times.entry(prev_phase).or_default() += duration;
        }
        self.current_phase_start = Some((phase, Instant::now()));
    }

    pub fn add_tool_output_lines(&mut self, tool_name: String, lines: usize) {
        *self.tool_output_lines.entry(tool_name).or_default() += lines;
    }

    pub fn start_approval(&mut self, summary: String) {
        self.plan_summary = summary;
        self.plan_summary_scroll = 0;
        self.approval_mode = ApprovalMode::AwaitingChoice;
        self.user_feedback.clear();
        self.cursor_position = 0;
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
