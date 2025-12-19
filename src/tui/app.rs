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

/// Which panel is focused for scrolling
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FocusedPanel {
    #[default]
    Output,
    Streaming,
}

pub struct App {
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub output_follow_mode: bool,  // true = auto-scroll to bottom, false = manual scroll
    pub streaming_lines: Vec<String>,
    pub streaming_scroll_position: usize,
    pub streaming_follow_mode: bool,  // true = auto-scroll to bottom, false = manual scroll
    pub focused_panel: FocusedPanel,  // which panel j/k scrolls
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

    // Streaming rate
    pub last_bytes_sample: (Instant, usize),
    pub bytes_per_second: f64,

    // Enhanced stats (new)
    pub turn_count: u32,
    pub model_name: Option<String>,
    pub last_stop_reason: Option<String>,
    pub tool_error_count: usize,
    pub total_tool_duration_ms: u64,
    pub completed_tool_count: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            output_lines: Vec::new(),
            scroll_position: 0,
            output_follow_mode: true,  // Start in follow mode
            streaming_lines: Vec::new(),
            streaming_scroll_position: 0,
            streaming_follow_mode: true,  // Start in follow mode
            focused_panel: FocusedPanel::default(),
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
            // Streaming rate
            last_bytes_sample: (Instant::now(), 0),
            bytes_per_second: 0.0,
            // Enhanced stats (new)
            turn_count: 0,
            model_name: None,
            last_stop_reason: None,
            tool_error_count: 0,
            total_tool_duration_ms: 0,
            completed_tool_count: 0,
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
        // Enable follow mode when new output arrives
        self.output_follow_mode = true;
    }

    pub fn scroll_up(&mut self) {
        // User took manual control - disable follow mode
        self.output_follow_mode = false;
        self.scroll_position = self.scroll_position.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        // Don't re-enable follow mode - user is still scrolling manually
        // UI will clamp to valid range
        self.scroll_position = self.scroll_position.saturating_add(1);
    }

    pub fn scroll_to_top(&mut self) {
        self.output_follow_mode = false;
        self.scroll_position = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        // Enable follow mode - UI will calculate the correct position
        self.output_follow_mode = true;
    }

    pub fn add_streaming(&mut self, line: String) {
        self.streaming_lines.push(line);
        // Enable follow mode when new streaming content arrives
        self.streaming_follow_mode = true;
    }

    pub fn streaming_scroll_up(&mut self) {
        // User took manual control - disable follow mode
        self.streaming_follow_mode = false;
        self.streaming_scroll_position = self.streaming_scroll_position.saturating_sub(1);
    }

    pub fn streaming_scroll_down(&mut self) {
        // Don't re-enable follow mode - user is still scrolling manually
        self.streaming_scroll_position = self.streaming_scroll_position.saturating_add(1);
    }

    pub fn streaming_scroll_to_bottom(&mut self) {
        // Enable follow mode - UI will calculate the correct position
        self.streaming_follow_mode = true;
    }

    pub fn toggle_focus(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Output => FocusedPanel::Streaming,
            FocusedPanel::Streaming => FocusedPanel::Output,
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_up_disables_follow_mode() {
        let mut app = App::new();
        app.add_output("line1".to_string());
        app.add_output("line2".to_string());
        assert!(app.output_follow_mode);

        app.scroll_up();
        assert!(!app.output_follow_mode);
    }

    #[test]
    fn test_scroll_to_bottom_enables_follow_mode() {
        let mut app = App::new();
        app.output_follow_mode = false;
        app.scroll_to_bottom();
        assert!(app.output_follow_mode);
    }

    #[test]
    fn test_add_output_enables_follow_mode() {
        let mut app = App::new();
        app.output_follow_mode = false;
        app.add_output("new line".to_string());
        assert!(app.output_follow_mode);
    }

    #[test]
    fn test_scroll_down_keeps_follow_mode_disabled() {
        let mut app = App::new();
        app.add_output("line1".to_string());
        app.scroll_up();  // Disable follow mode
        assert!(!app.output_follow_mode);

        app.scroll_down();
        assert!(!app.output_follow_mode);
    }

    #[test]
    fn test_streaming_scroll_up_disables_follow_mode() {
        let mut app = App::new();
        app.add_streaming("line1".to_string());
        assert!(app.streaming_follow_mode);

        app.streaming_scroll_up();
        assert!(!app.streaming_follow_mode);
    }

    #[test]
    fn test_streaming_scroll_to_bottom_enables_follow_mode() {
        let mut app = App::new();
        app.streaming_follow_mode = false;
        app.streaming_scroll_to_bottom();
        assert!(app.streaming_follow_mode);
    }

    #[test]
    fn test_toggle_focus() {
        let mut app = App::new();
        assert_eq!(app.focused_panel, FocusedPanel::Output);

        app.toggle_focus();
        assert_eq!(app.focused_panel, FocusedPanel::Streaming);

        app.toggle_focus();
        assert_eq!(app.focused_panel, FocusedPanel::Output);
    }

    #[test]
    fn test_scroll_to_top_disables_follow_mode() {
        let mut app = App::new();
        app.add_output("line1".to_string());
        assert!(app.output_follow_mode);

        app.scroll_to_top();
        assert!(!app.output_follow_mode);
        assert_eq!(app.scroll_position, 0);
    }
}
