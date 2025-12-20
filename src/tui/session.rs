use crate::state::{Phase, State};
use crate::tui::event::TokenUsage;
use crate::WorkflowResult;
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::event::UserApprovalResponse;

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

/// Input mode for the session
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    /// User is entering objective for new tab
    NamingTab,
}

/// Status of a session for display in tab bar
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    InputPending,
    Planning,
    AwaitingApproval,
    Complete,
    Error,
}

/// An independent planning session that can run in a tab
#[allow(dead_code)]
pub struct Session {
    pub id: usize,
    pub name: String,
    pub status: SessionStatus,

    // Output/display state
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub output_follow_mode: bool,
    pub streaming_lines: Vec<String>,
    pub streaming_scroll_position: usize,
    pub streaming_follow_mode: bool,
    pub focused_panel: FocusedPanel,

    // Workflow state
    pub workflow_state: Option<State>,
    pub start_time: Instant,
    pub total_cost: f64,
    pub running: bool,
    pub active_tools: Vec<(String, Instant)>,

    // User approval state
    pub approval_mode: ApprovalMode,
    pub plan_summary: String,
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,

    // Input mode for tab naming
    pub input_mode: InputMode,
    pub tab_input: String,
    pub tab_input_cursor: usize,
    pub tab_input_scroll: usize,

    // Error state
    pub error_state: Option<String>,

    // Stats
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

    // Session-specific workflow handles
    pub workflow_handle: Option<JoinHandle<Result<WorkflowResult>>>,
    pub approval_tx: Option<mpsc::Sender<UserApprovalResponse>>,
}

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
            active_tools: Vec::new(),

            approval_mode: ApprovalMode::None,
            plan_summary: String::new(),
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,

            input_mode: InputMode::Normal,
            tab_input: String::new(),
            tab_input_cursor: 0,
            tab_input_scroll: 0,

            error_state: None,

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
        }
    }

    /// Create a session with a pre-set name (for the initial CLI session)
    pub fn with_name(id: usize, name: String) -> Self {
        let mut session = Self::new(id);
        session.name = name;
        session.input_mode = InputMode::Normal;
        session.status = SessionStatus::Planning;
        session
    }

    pub fn handle_error(&mut self, error: &str) {
        self.error_state = Some(error.to_string());
        self.workflow_handle = None;
        self.status = SessionStatus::Error;
    }

    pub fn clear_error(&mut self) {
        self.error_state = None;
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
        self.output_follow_mode = true;
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
    }

    pub fn add_streaming(&mut self, line: String) {
        self.streaming_lines.push(line);
        self.streaming_follow_mode = true;
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
        }
    }

    /// Insert a character into the tab input buffer
    pub fn insert_tab_input_char(&mut self, c: char) {
        self.tab_input.insert(self.tab_input_cursor, c);
        self.tab_input_cursor += c.len_utf8();
    }

    /// Delete a character from the tab input buffer
    pub fn delete_tab_input_char(&mut self) {
        if self.tab_input_cursor > 0 {
            // Find the previous character boundary
            let prev_char_boundary = self.tab_input[..self.tab_input_cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.tab_input.remove(prev_char_boundary);
            self.tab_input_cursor = prev_char_boundary;
        }
    }

    /// Move tab input cursor left
    pub fn move_tab_input_cursor_left(&mut self) {
        if self.tab_input_cursor > 0 {
            // Find the previous character boundary
            self.tab_input_cursor = self.tab_input[..self.tab_input_cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    /// Move tab input cursor right
    pub fn move_tab_input_cursor_right(&mut self) {
        if self.tab_input_cursor < self.tab_input.len() {
            // Find the next character boundary
            if let Some((_, c)) = self.tab_input[self.tab_input_cursor..].char_indices().next() {
                self.tab_input_cursor += c.len_utf8();
            }
        }
    }

    /// Insert a newline into the tab input buffer
    pub fn insert_tab_input_newline(&mut self) {
        self.tab_input.insert(self.tab_input_cursor, '\n');
        self.tab_input_cursor += '\n'.len_utf8();
    }

    /// Move tab input cursor up to the previous line
    pub fn move_tab_input_cursor_up(&mut self) {
        let text_before = &self.tab_input[..self.tab_input_cursor];

        // Find the start of the current line
        let current_line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);

        // If we're on the first line, do nothing
        if current_line_start == 0 {
            return;
        }

        // Display width column position in current line
        let display_col = self.tab_input[current_line_start..self.tab_input_cursor].width();

        // Find the start of the previous line
        let prev_line_end = current_line_start - 1; // Position of the '\n' before current line
        let prev_line_start = self.tab_input[..prev_line_end]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);

        // Find byte position in previous line that corresponds to target display column
        let prev_line = &self.tab_input[prev_line_start..prev_line_end];
        let mut accumulated_width = 0;
        let mut target_byte_offset = prev_line.len(); // Default to end of line
        for (idx, c) in prev_line.char_indices() {
            if accumulated_width >= display_col {
                target_byte_offset = idx;
                break;
            }
            accumulated_width += c.width().unwrap_or(0);
        }

        self.tab_input_cursor = prev_line_start + target_byte_offset;
    }

    /// Move tab input cursor down to the next line
    pub fn move_tab_input_cursor_down(&mut self) {
        let text_before = &self.tab_input[..self.tab_input_cursor];
        let text_after = &self.tab_input[self.tab_input_cursor..];

        // Find the start of the current line
        let current_line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);

        // Display width column position in current line
        let display_col = self.tab_input[current_line_start..self.tab_input_cursor].width();

        // Find the end of current line (next newline after cursor)
        let next_newline = text_after.find('\n');

        // If there's no next line, do nothing
        let Some(offset) = next_newline else {
            return;
        };

        // Start of next line
        let next_line_start = self.tab_input_cursor + offset + 1;

        // Find end of next line
        let next_line_end = self.tab_input[next_line_start..]
            .find('\n')
            .map(|p| next_line_start + p)
            .unwrap_or(self.tab_input.len());

        // Find byte position in next line that corresponds to target display column
        let next_line = &self.tab_input[next_line_start..next_line_end];
        let mut accumulated_width = 0;
        let mut target_byte_offset = next_line.len(); // Default to end of line
        for (idx, c) in next_line.char_indices() {
            if accumulated_width >= display_col {
                target_byte_offset = idx;
                break;
            }
            accumulated_width += c.width().unwrap_or(0);
        }

        self.tab_input_cursor = next_line_start + target_byte_offset;
    }

    /// Get the current line number and column for the tab input cursor
    /// Returns (line_number, display_column) where display_column is the unicode width
    pub fn get_tab_input_cursor_position(&self) -> (usize, usize) {
        let text_before = &self.tab_input[..self.tab_input_cursor];
        let line = text_before.matches('\n').count();
        let line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        // Use display width for column, not byte offset
        let col = self.tab_input[line_start..self.tab_input_cursor].width();
        (line, col)
    }

    /// Get the total number of lines in the tab input
    pub fn get_tab_input_line_count(&self) -> usize {
        self.tab_input.matches('\n').count() + 1
    }

    /// Get the display cost and its source indicator
    /// Returns (cost, source_label) where source_label is "API" or "est"
    pub fn display_cost(&self) -> (f64, &'static str) {
        if self.total_cost > 0.0 {
            (self.total_cost, "API")
        } else {
            (self.estimated_cost(), "est")
        }
    }

    /// Calculate estimated cost based on model and token usage
    /// Uses API-provided total_cost if available, otherwise calculates from token counts
    /// Pricing per 1M tokens (as of Jan 2025):
    /// - Opus 4: Input $15, Output $75, Cache read $1.50, Cache write $18.75
    /// - Sonnet 3.5: Input $3, Output $15, Cache read $0.30, Cache write $3.75
    /// - Haiku 3.5: Input $0.80, Output $4, Cache read $0.08, Cache write $1
    pub fn estimated_cost(&self) -> f64 {
        // Use API-provided cost if available
        if self.total_cost > 0.0 {
            return self.total_cost;
        }

        // Calculate from token counts
        let (input_rate, output_rate, cache_read_rate, cache_write_rate) =
            match self.model_name.as_deref() {
                Some(m) if m.contains("opus") => (15.0, 75.0, 1.50, 18.75),
                Some(m) if m.contains("sonnet") => (3.0, 15.0, 0.30, 3.75),
                Some(m) if m.contains("haiku") => (0.80, 4.0, 0.08, 1.0),
                _ => (3.0, 15.0, 0.30, 3.75), // Default to Sonnet pricing
            };

        let million = 1_000_000.0;
        (self.total_input_tokens as f64 * input_rate / million)
            + (self.total_output_tokens as f64 * output_rate / million)
            + (self.total_cache_read_tokens as f64 * cache_read_rate / million)
            + (self.total_cache_creation_tokens as f64 * cache_write_rate / million)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_up_disables_follow_mode() {
        let mut session = Session::new(0);
        session.add_output("line1".to_string());
        session.add_output("line2".to_string());
        assert!(session.output_follow_mode);

        session.scroll_up();
        assert!(!session.output_follow_mode);
    }

    #[test]
    fn test_scroll_to_bottom_enables_follow_mode() {
        let mut session = Session::new(0);
        session.output_follow_mode = false;
        session.scroll_to_bottom();
        assert!(session.output_follow_mode);
    }

    #[test]
    fn test_add_output_enables_follow_mode() {
        let mut session = Session::new(0);
        session.output_follow_mode = false;
        session.add_output("new line".to_string());
        assert!(session.output_follow_mode);
    }

    #[test]
    fn test_toggle_focus() {
        let mut session = Session::new(0);
        assert_eq!(session.focused_panel, FocusedPanel::Output);

        session.toggle_focus();
        assert_eq!(session.focused_panel, FocusedPanel::Streaming);

        session.toggle_focus();
        assert_eq!(session.focused_panel, FocusedPanel::Output);
    }

    #[test]
    fn test_input_mode_transitions() {
        let mut session = Session::new(0);
        assert_eq!(session.input_mode, InputMode::Normal);

        session.input_mode = InputMode::NamingTab;
        assert_eq!(session.input_mode, InputMode::NamingTab);

        session.input_mode = InputMode::Normal;
        assert_eq!(session.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_tab_input_buffer() {
        let mut session = Session::new(0);
        session.input_mode = InputMode::NamingTab;

        session.insert_tab_input_char('h');
        session.insert_tab_input_char('e');
        session.insert_tab_input_char('l');
        session.insert_tab_input_char('l');
        session.insert_tab_input_char('o');

        assert_eq!(session.tab_input, "hello");
        assert_eq!(session.tab_input_cursor, 5);

        session.delete_tab_input_char();
        assert_eq!(session.tab_input, "hell");
        assert_eq!(session.tab_input_cursor, 4);
    }

    #[test]
    fn test_session_with_name() {
        let session = Session::with_name(1, "test-feature".to_string());
        assert_eq!(session.id, 1);
        assert_eq!(session.name, "test-feature");
        assert_eq!(session.input_mode, InputMode::Normal);
        assert_eq!(session.status, SessionStatus::Planning);
    }

    #[test]
    fn test_insert_newline() {
        let mut session = Session::new(0);
        session.tab_input = "hello".to_string();
        session.tab_input_cursor = 5;

        session.insert_tab_input_newline();

        assert_eq!(session.tab_input, "hello\n");
        assert_eq!(session.tab_input_cursor, 6);

        // Insert newline in the middle
        session.tab_input = "hello world".to_string();
        session.tab_input_cursor = 5;
        session.insert_tab_input_newline();

        assert_eq!(session.tab_input, "hello\n world");
        assert_eq!(session.tab_input_cursor, 6);
    }

    #[test]
    fn test_cursor_up_movement() {
        let mut session = Session::new(0);
        session.tab_input = "line1\nline2\nline3".to_string();
        session.tab_input_cursor = 14; // At 'n' in "line3"

        session.move_tab_input_cursor_up();
        assert_eq!(session.tab_input_cursor, 8); // At 'n' in "line2"

        session.move_tab_input_cursor_up();
        assert_eq!(session.tab_input_cursor, 2); // At 'n' in "line1"
    }

    #[test]
    fn test_cursor_down_movement() {
        let mut session = Session::new(0);
        session.tab_input = "line1\nline2\nline3".to_string();
        session.tab_input_cursor = 2; // At 'n' in "line1"

        session.move_tab_input_cursor_down();
        assert_eq!(session.tab_input_cursor, 8); // At 'n' in "line2"

        session.move_tab_input_cursor_down();
        assert_eq!(session.tab_input_cursor, 14); // At 'n' in "line3"
    }

    #[test]
    fn test_cursor_up_at_first_line() {
        let mut session = Session::new(0);
        session.tab_input = "line1\nline2".to_string();
        session.tab_input_cursor = 2; // At 'n' in "line1"

        session.move_tab_input_cursor_up();
        assert_eq!(session.tab_input_cursor, 2); // Should stay at same position
    }

    #[test]
    fn test_cursor_down_at_last_line() {
        let mut session = Session::new(0);
        session.tab_input = "line1\nline2".to_string();
        session.tab_input_cursor = 8; // At 'n' in "line2"

        session.move_tab_input_cursor_down();
        assert_eq!(session.tab_input_cursor, 8); // Should stay at same position
    }

    #[test]
    fn test_cursor_up_clamps_to_shorter_line() {
        let mut session = Session::new(0);
        session.tab_input = "hi\nworld".to_string();
        session.tab_input_cursor = 7; // At 'l' in "world" (col 4)

        session.move_tab_input_cursor_up();
        assert_eq!(session.tab_input_cursor, 2); // Clamped to end of "hi" (col 2)
    }

    #[test]
    fn test_cursor_down_clamps_to_shorter_line() {
        let mut session = Session::new(0);
        session.tab_input = "world\nhi".to_string();
        session.tab_input_cursor = 4; // At 'l' in "world" (col 4)

        session.move_tab_input_cursor_down();
        assert_eq!(session.tab_input_cursor, 8); // Clamped to end of "hi" (col 2)
    }

    #[test]
    fn test_get_tab_input_cursor_position() {
        let mut session = Session::new(0);
        session.tab_input = "line1\nline2\nline3".to_string();

        session.tab_input_cursor = 0;
        assert_eq!(session.get_tab_input_cursor_position(), (0, 0));

        session.tab_input_cursor = 3;
        assert_eq!(session.get_tab_input_cursor_position(), (0, 3));

        session.tab_input_cursor = 6; // Start of line2
        assert_eq!(session.get_tab_input_cursor_position(), (1, 0));

        session.tab_input_cursor = 14; // At 'n' in line3
        assert_eq!(session.get_tab_input_cursor_position(), (2, 2));
    }

    #[test]
    fn test_get_tab_input_line_count() {
        let mut session = Session::new(0);

        session.tab_input = "single line".to_string();
        assert_eq!(session.get_tab_input_line_count(), 1);

        session.tab_input = "line1\nline2".to_string();
        assert_eq!(session.get_tab_input_line_count(), 2);

        session.tab_input = "line1\nline2\nline3".to_string();
        assert_eq!(session.get_tab_input_line_count(), 3);

        session.tab_input = "".to_string();
        assert_eq!(session.get_tab_input_line_count(), 1);
    }

    #[test]
    fn test_display_cost_returns_api_cost_when_available() {
        let mut session = Session::new(0);
        session.total_cost = 0.1234;
        session.total_input_tokens = 1000;
        session.total_output_tokens = 500;

        let (cost, source) = session.display_cost();
        assert_eq!(cost, 0.1234);
        assert_eq!(source, "API");
    }

    #[test]
    fn test_display_cost_falls_back_to_estimated() {
        let mut session = Session::new(0);
        session.total_cost = 0.0;
        session.total_input_tokens = 1000;
        session.total_output_tokens = 500;
        session.model_name = Some("claude-3.5-sonnet".to_string());

        let (cost, source) = session.display_cost();
        // Should return estimated cost, not 0
        assert!(cost > 0.0);
        assert_eq!(source, "est");
    }
}
