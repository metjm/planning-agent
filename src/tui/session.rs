use crate::claude_usage::ClaudeUsage;
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

/// A single chat message from an agent
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub agent_name: String,
    pub message: String,
    pub timestamp: Instant,
}

/// A run tab containing messages for a specific phase
#[derive(Debug, Clone)]
pub struct RunTab {
    pub phase: String,           // "Planning", "Reviewing #1", "Revising #1", etc.
    pub messages: Vec<ChatMessage>,
    pub scroll_position: usize,  // Per-tab scroll state
}

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

/// What kind of approval prompt is being shown
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ApprovalContext {
    #[default]
    PlanApproval,
    ReviewDecision,
}

/// Which panel is focused for scrolling
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FocusedPanel {
    #[default]
    Output,
    Chat,  // Renamed from Streaming
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
    GeneratingSummary,
    AwaitingApproval,
    Complete,
    Error,
}

/// Represents a pasted text block at a specific position
#[derive(Debug, Clone)]
pub struct PasteBlock {
    /// The original pasted content
    pub content: String,
    /// Byte position in the input where this paste starts
    pub start_pos: usize,
    /// Number of lines in the pasted content
    pub line_count: usize,
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
    pub approval_context: ApprovalContext,
    pub plan_summary: String,
    pub plan_summary_scroll: usize,
    pub user_feedback: String,
    pub cursor_position: usize,

    // Input mode for tab naming
    pub input_mode: InputMode,
    pub tab_input: String,
    pub tab_input_cursor: usize,
    pub tab_input_scroll: usize,

    // Track last key for Shift+Enter detection (some terminals send backslash before Enter)
    pub last_key_was_backslash: bool,

    // Paste tracking for tab input
    pub tab_input_pastes: Vec<PasteBlock>,

    // Paste tracking for feedback input
    pub feedback_pastes: Vec<PasteBlock>,

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

    // Claude account usage (shared across sessions)
    pub claude_usage: ClaudeUsage,

    /// Spinner frame counter for generating summary animation
    pub spinner_frame: u8,

    // Chat tabs system (replaces streaming_lines for the chat view)
    pub run_tabs: Vec<RunTab>,
    pub active_run_tab: usize,
    pub chat_follow_mode: bool,  // Auto-scroll to bottom on new messages
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
            approval_context: ApprovalContext::PlanApproval,
            plan_summary: String::new(),
            plan_summary_scroll: 0,
            user_feedback: String::new(),
            cursor_position: 0,

            input_mode: InputMode::Normal,
            tab_input: String::new(),
            tab_input_cursor: 0,
            tab_input_scroll: 0,
            last_key_was_backslash: false,

            tab_input_pastes: Vec::new(),
            feedback_pastes: Vec::new(),

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

            claude_usage: ClaudeUsage::default(),

            spinner_frame: 0,

            run_tabs: Vec::new(),
            active_run_tab: 0,
            chat_follow_mode: true,
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
        self.cursor_position += c.len_utf8();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            let prev_char_boundary = self.user_feedback[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.user_feedback.remove(prev_char_boundary);
            self.cursor_position = prev_char_boundary;
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            self.cursor_position = self.user_feedback[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.user_feedback.len() {
            // Find the next character boundary
            if let Some((_, c)) = self.user_feedback[self.cursor_position..].char_indices().next() {
                self.cursor_position += c.len_utf8();
            }
        }
    }

    /// Insert a newline into the feedback input buffer
    pub fn insert_feedback_newline(&mut self) {
        self.user_feedback.insert(self.cursor_position, '\n');
        self.cursor_position += '\n'.len_utf8();
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
        // Atomically set follow mode and update scroll position to end
        self.output_follow_mode = true;
        self.scroll_position = self.output_lines.len().saturating_sub(1);
    }

    pub fn scroll_up(&mut self) {
        // Disable follow mode BEFORE updating position for consistency
        self.output_follow_mode = false;
        self.scroll_position = self.scroll_position.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        // When scrolling down, don't auto-enable follow mode - user is still in control
        self.scroll_position = self.scroll_position.saturating_add(1);
    }

    pub fn scroll_to_top(&mut self) {
        self.output_follow_mode = false;
        self.scroll_position = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        // Enable follow mode and sync position
        self.output_follow_mode = true;
        self.scroll_position = self.output_lines.len().saturating_sub(1);
    }

    pub fn add_streaming(&mut self, line: String) {
        self.streaming_lines.push(line);
        // Atomically set follow mode and update scroll position to end
        self.streaming_follow_mode = true;
        self.streaming_scroll_position = self.streaming_lines.len().saturating_sub(1);
    }

    pub fn streaming_scroll_up(&mut self) {
        // Disable follow mode BEFORE updating position for consistency
        self.streaming_follow_mode = false;
        self.streaming_scroll_position = self.streaming_scroll_position.saturating_sub(1);
    }

    pub fn streaming_scroll_down(&mut self) {
        // When scrolling down, don't auto-enable follow mode - user is still in control
        self.streaming_scroll_position = self.streaming_scroll_position.saturating_add(1);
    }

    pub fn streaming_scroll_to_bottom(&mut self) {
        // Enable follow mode and sync position
        self.streaming_follow_mode = true;
        self.streaming_scroll_position = self.streaming_lines.len().saturating_sub(1);
    }

    pub fn toggle_focus(&mut self) {
        self.focused_panel = match self.focused_panel {
            FocusedPanel::Output => FocusedPanel::Chat,
            FocusedPanel::Chat => FocusedPanel::Output,
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

    // ===== Chat tabs methods =====

    /// Create a new run tab for a phase
    pub fn add_run_tab(&mut self, phase: String) {
        self.run_tabs.push(RunTab {
            phase,
            messages: Vec::new(),
            scroll_position: 0,
        });
        // Auto-switch to newest tab
        self.active_run_tab = self.run_tabs.len().saturating_sub(1);
    }

    /// Add a chat message to the appropriate tab (creates tab if none exists for phase)
    pub fn add_chat_message(&mut self, agent_name: &str, phase: &str, message: String) {
        // Skip empty/whitespace messages
        if message.trim().is_empty() {
            return;
        }

        // Find or create tab for this phase
        let tab_idx = self.run_tabs.iter().position(|t| t.phase == phase);
        let idx = match tab_idx {
            Some(i) => i,
            None => {
                self.add_run_tab(phase.to_string());
                self.run_tabs.len() - 1
            }
        };

        self.run_tabs[idx].messages.push(ChatMessage {
            agent_name: agent_name.to_string(),
            message,
            timestamp: Instant::now(),
        });

        // Auto-scroll if in follow mode
        if self.chat_follow_mode {
            // This will be calculated during render
        }
    }

    pub fn next_run_tab(&mut self) {
        if self.active_run_tab < self.run_tabs.len().saturating_sub(1) {
            self.active_run_tab += 1;
        }
    }

    pub fn prev_run_tab(&mut self) {
        self.active_run_tab = self.active_run_tab.saturating_sub(1);
    }

    /// Reset chat tabs (call on workflow restart)
    pub fn clear_chat_tabs(&mut self) {
        self.run_tabs.clear();
        self.active_run_tab = 0;
        self.chat_follow_mode = true;
    }

    /// Scroll the active chat tab up
    pub fn chat_scroll_up(&mut self) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.scroll_position = tab.scroll_position.saturating_sub(1);
            self.chat_follow_mode = false;
        }
    }

    /// Scroll the active chat tab down
    pub fn chat_scroll_down(&mut self) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.scroll_position = tab.scroll_position.saturating_add(1);
        }
    }

    /// Scroll to bottom and enable follow mode for chat
    pub fn chat_scroll_to_bottom(&mut self) {
        self.chat_follow_mode = true;
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

    /// Get the cursor position for feedback input with proper unicode display width handling
    /// Returns (row, col) where col is the display width column position
    pub fn get_feedback_cursor_position(&self, width: usize) -> (usize, usize) {
        if width == 0 {
            return (0, 0);
        }

        let text_before = &self.user_feedback[..self.cursor_position.min(self.user_feedback.len())];
        let mut row = 0;
        let mut col = 0;

        for c in text_before.chars() {
            if c == '\n' {
                row += 1;
                col = 0;
            } else {
                let char_width = c.width().unwrap_or(0);
                // Check if adding this character would exceed width (causing a wrap)
                if col + char_width > width && col > 0 {
                    row += 1;
                    col = char_width;
                } else {
                    col += char_width;
                }
            }
        }

        (row, col)
    }

    /// Get the display cost (API-provided only)
    pub fn display_cost(&self) -> f64 {
        self.total_cost
    }

    // ===== Paste handling methods =====

    /// Insert a paste block into the tab input at the current cursor position
    pub fn insert_paste_tab_input(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        // Normalize line endings: \r\n -> \n, \r -> \n
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = normalized.lines().count().max(1);

        // Create paste block at current cursor position
        let paste_block = PasteBlock {
            content: text,
            start_pos: self.tab_input_cursor,
            line_count,
        };

        // Insert placeholder marker into the visible text
        // We use a zero-width space followed by a special marker character
        let placeholder = Self::format_paste_placeholder(line_count);
        self.tab_input.insert_str(self.tab_input_cursor, &placeholder);
        self.tab_input_cursor += placeholder.len();

        // Update start positions of any pastes that come after this one
        for paste in &mut self.tab_input_pastes {
            if paste.start_pos >= paste_block.start_pos {
                paste.start_pos += placeholder.len();
            }
        }

        self.tab_input_pastes.push(paste_block);
    }

    /// Insert a paste block into the feedback input at the current cursor position
    pub fn insert_paste_feedback(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        // Normalize line endings: \r\n -> \n, \r -> \n
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = normalized.lines().count().max(1);

        // Create paste block at current cursor position
        let paste_block = PasteBlock {
            content: text,
            start_pos: self.cursor_position,
            line_count,
        };

        // Insert placeholder marker into the visible text
        let placeholder = Self::format_paste_placeholder(line_count);
        self.user_feedback.insert_str(self.cursor_position, &placeholder);
        self.cursor_position += placeholder.len();

        // Update start positions of any pastes that come after this one
        for paste in &mut self.feedback_pastes {
            if paste.start_pos >= paste_block.start_pos {
                paste.start_pos += placeholder.len();
            }
        }

        self.feedback_pastes.push(paste_block);
    }

    /// Format the paste placeholder text
    fn format_paste_placeholder(line_count: usize) -> String {
        if line_count <= 1 {
            "[Pasted]".to_string()
        } else {
            format!("[Pasted +{} lines]", line_count - 1)
        }
    }

    /// Delete paste block at cursor position for tab input, returns true if a paste was deleted
    pub fn delete_paste_at_cursor_tab(&mut self) -> bool {
        // Check if cursor is within or at the end of any paste placeholder
        if let Some(idx) = self.find_paste_at_cursor_tab() {
            let paste = self.tab_input_pastes.remove(idx);
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_len = placeholder.len();

            // Remove the placeholder from the text
            self.tab_input = format!(
                "{}{}",
                &self.tab_input[..paste.start_pos],
                &self.tab_input[paste.start_pos + placeholder_len..]
            );

            // Move cursor to start of where paste was
            self.tab_input_cursor = paste.start_pos;

            // Update start positions of any pastes that came after
            for p in &mut self.tab_input_pastes {
                if p.start_pos > paste.start_pos {
                    p.start_pos -= placeholder_len;
                }
            }

            return true;
        }
        false
    }

    /// Delete paste block at cursor position for feedback input, returns true if a paste was deleted
    pub fn delete_paste_at_cursor_feedback(&mut self) -> bool {
        // Check if cursor is within or at the end of any paste placeholder
        if let Some(idx) = self.find_paste_at_cursor_feedback() {
            let paste = self.feedback_pastes.remove(idx);
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_len = placeholder.len();

            // Remove the placeholder from the text
            self.user_feedback = format!(
                "{}{}",
                &self.user_feedback[..paste.start_pos],
                &self.user_feedback[paste.start_pos + placeholder_len..]
            );

            // Move cursor to start of where paste was
            self.cursor_position = paste.start_pos;

            // Update start positions of any pastes that came after
            for p in &mut self.feedback_pastes {
                if p.start_pos > paste.start_pos {
                    p.start_pos -= placeholder_len;
                }
            }

            return true;
        }
        false
    }

    /// Find paste block index at cursor position for tab input
    fn find_paste_at_cursor_tab(&self) -> Option<usize> {
        for (idx, paste) in self.tab_input_pastes.iter().enumerate() {
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_end = paste.start_pos + placeholder.len();

            // Check if cursor is at end of placeholder (backspace case)
            // or within the placeholder
            if self.tab_input_cursor > paste.start_pos
                && self.tab_input_cursor <= placeholder_end
            {
                return Some(idx);
            }
        }
        None
    }

    /// Find paste block index at cursor position for feedback input
    fn find_paste_at_cursor_feedback(&self) -> Option<usize> {
        for (idx, paste) in self.feedback_pastes.iter().enumerate() {
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_end = paste.start_pos + placeholder.len();

            // Check if cursor is at end of placeholder (backspace case)
            // or within the placeholder
            if self.cursor_position > paste.start_pos
                && self.cursor_position <= placeholder_end
            {
                return Some(idx);
            }
        }
        None
    }

    /// Get display text for tab input with placeholders shown
    pub fn get_display_text_tab(&self) -> String {
        // The tab_input already contains placeholders, just return it
        self.tab_input.clone()
    }

    /// Get display text for feedback input with placeholders shown
    pub fn get_display_text_feedback(&self) -> String {
        // The user_feedback already contains placeholders, just return it
        self.user_feedback.clone()
    }

    /// Get full text for submission with pastes expanded (tab input)
    pub fn get_submit_text_tab(&self) -> String {
        self.expand_pastes_in_text(&self.tab_input, &self.tab_input_pastes)
    }

    /// Get full text for submission with pastes expanded (feedback input)
    pub fn get_submit_text_feedback(&self) -> String {
        self.expand_pastes_in_text(&self.user_feedback, &self.feedback_pastes)
    }

    /// Expand paste placeholders in text with their actual content
    fn expand_pastes_in_text(&self, text: &str, pastes: &[PasteBlock]) -> String {
        if pastes.is_empty() {
            return text.to_string();
        }

        // Sort pastes by position (in reverse to process from end to start)
        let mut sorted_pastes: Vec<_> = pastes.iter().collect();
        sorted_pastes.sort_by(|a, b| b.start_pos.cmp(&a.start_pos));

        let mut result = text.to_string();

        for paste in sorted_pastes {
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_end = paste.start_pos + placeholder.len();

            if placeholder_end <= result.len() {
                result = format!(
                    "{}{}{}",
                    &result[..paste.start_pos],
                    &paste.content,
                    &result[placeholder_end..]
                );
            }
        }

        result
    }

    /// Clear paste blocks for tab input (call when clearing the input)
    pub fn clear_tab_input_pastes(&mut self) {
        self.tab_input_pastes.clear();
    }

    /// Clear paste blocks for feedback input (call when clearing the input)
    pub fn clear_feedback_pastes(&mut self) {
        self.feedback_pastes.clear();
    }

    /// Check if tab input has any paste blocks
    pub fn has_tab_input_pastes(&self) -> bool {
        !self.tab_input_pastes.is_empty()
    }

    /// Check if feedback input has any paste blocks
    pub fn has_feedback_pastes(&self) -> bool {
        !self.feedback_pastes.is_empty()
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
        assert_eq!(session.focused_panel, FocusedPanel::Chat);

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
    fn test_display_cost_returns_api_cost() {
        let mut session = Session::new(0);
        session.total_cost = 0.1234;
        session.total_input_tokens = 1000;
        session.total_output_tokens = 500;

        let cost = session.display_cost();
        assert_eq!(cost, 0.1234);
    }

    #[test]
    fn test_display_cost_returns_zero_when_no_api_cost() {
        let mut session = Session::new(0);
        session.total_cost = 0.0;
        session.total_input_tokens = 1000;
        session.total_output_tokens = 500;
        session.model_name = Some("claude-3.5-sonnet".to_string());

        let cost = session.display_cost();
        // Should return 0.0, not an estimated cost
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_feedback_cursor_position_basic() {
        let mut session = Session::new(0);
        session.user_feedback = "hello".to_string();
        session.cursor_position = 5;

        // At width 10, "hello" fits on one line
        let (row, col) = session.get_feedback_cursor_position(10);
        assert_eq!(row, 0);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_feedback_cursor_position_with_wrap() {
        let mut session = Session::new(0);
        session.user_feedback = "hello world".to_string();
        session.cursor_position = 11; // At end of "hello world"

        // At width 5, text wraps: "hello" | " worl" | "d"
        let (row, col) = session.get_feedback_cursor_position(5);
        // "hello" = row 0 (5 chars), " worl" = row 1 (5 chars), "d" = row 2 (1 char)
        assert_eq!(row, 2);
        assert_eq!(col, 1);
    }

    #[test]
    fn test_feedback_cursor_position_empty() {
        let mut session = Session::new(0);
        session.user_feedback = "".to_string();
        session.cursor_position = 0;

        let (row, col) = session.get_feedback_cursor_position(10);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_feedback_cursor_with_newline() {
        let mut session = Session::new(0);
        session.user_feedback = "hello\nworld".to_string();
        session.cursor_position = 8; // At "or" in "world"

        let (row, col) = session.get_feedback_cursor_position(20);
        assert_eq!(row, 1);
        assert_eq!(col, 2);
    }

    #[test]
    fn test_feedback_insert_char_utf8() {
        let mut session = Session::new(0);
        session.user_feedback = "".to_string();
        session.cursor_position = 0;

        session.insert_char('你');
        assert_eq!(session.user_feedback, "你");
        assert_eq!(session.cursor_position, 3); // '你' is 3 bytes in UTF-8

        session.insert_char('好');
        assert_eq!(session.user_feedback, "你好");
        assert_eq!(session.cursor_position, 6);
    }

    #[test]
    fn test_feedback_delete_char_utf8() {
        let mut session = Session::new(0);
        session.user_feedback = "你好".to_string();
        session.cursor_position = 6; // At end

        session.delete_char();
        assert_eq!(session.user_feedback, "你");
        assert_eq!(session.cursor_position, 3);

        session.delete_char();
        assert_eq!(session.user_feedback, "");
        assert_eq!(session.cursor_position, 0);
    }

    #[test]
    fn test_add_output_syncs_scroll() {
        let mut session = Session::new(0);
        session.output_follow_mode = false;
        session.scroll_position = 0;

        session.add_output("line1".to_string());
        assert!(session.output_follow_mode);
        assert_eq!(session.scroll_position, 0); // 1 line, saturating_sub(1) = 0

        session.add_output("line2".to_string());
        assert_eq!(session.scroll_position, 1); // 2 lines, saturating_sub(1) = 1
    }

    #[test]
    fn test_scroll_to_bottom_syncs_position() {
        let mut session = Session::new(0);
        session.output_lines = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        session.scroll_position = 0;
        session.output_follow_mode = false;

        session.scroll_to_bottom();
        assert!(session.output_follow_mode);
        assert_eq!(session.scroll_position, 2); // 3 lines, position at 2
    }

    // ===== Paste handling tests =====

    #[test]
    fn test_insert_paste_basic() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("hello world".to_string());

        assert!(session.has_tab_input_pastes());
        assert_eq!(session.tab_input_pastes.len(), 1);
        assert_eq!(session.tab_input, "[Pasted]");
        assert_eq!(session.tab_input_cursor, 8);
    }

    #[test]
    fn test_insert_paste_multiline() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("line1\nline2\nline3".to_string());

        assert_eq!(session.tab_input_pastes.len(), 1);
        assert_eq!(session.tab_input_pastes[0].line_count, 3);
        assert_eq!(session.tab_input, "[Pasted +2 lines]");
    }

    #[test]
    fn test_get_display_text_with_paste() {
        let mut session = Session::new(0);
        session.tab_input = "prefix ".to_string();
        session.tab_input_cursor = 7;
        session.insert_paste_tab_input("pasted content\nmore content".to_string());

        let display = session.get_display_text_tab();
        assert_eq!(display, "prefix [Pasted +1 lines]");
    }

    #[test]
    fn test_get_submit_text_expands_paste() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("pasted content".to_string());

        let submit = session.get_submit_text_tab();
        assert_eq!(submit, "pasted content");
    }

    #[test]
    fn test_get_submit_text_with_surrounding_text() {
        let mut session = Session::new(0);
        session.tab_input = "before ".to_string();
        session.tab_input_cursor = 7;
        session.insert_paste_tab_input("pasted".to_string());
        session.tab_input.push_str(" after");

        let submit = session.get_submit_text_tab();
        assert_eq!(submit, "before pasted after");
    }

    #[test]
    fn test_delete_paste_block() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("content".to_string());

        assert!(session.has_tab_input_pastes());
        assert!(session.delete_paste_at_cursor_tab());
        assert!(!session.has_tab_input_pastes());
        assert!(session.tab_input.is_empty());
        assert_eq!(session.tab_input_cursor, 0);
    }

    #[test]
    fn test_multiple_pastes() {
        let mut session = Session::new(0);

        session.insert_paste_tab_input("first".to_string());
        session.tab_input.push(' ');
        session.tab_input_cursor = session.tab_input.len();
        session.insert_paste_tab_input("second".to_string());

        assert_eq!(session.tab_input_pastes.len(), 2);
        assert_eq!(session.tab_input, "[Pasted] [Pasted]");

        let submit = session.get_submit_text_tab();
        assert_eq!(submit, "first second");
    }

    #[test]
    fn test_feedback_paste_insert_and_expand() {
        let mut session = Session::new(0);
        session.insert_paste_feedback("feedback content".to_string());

        assert!(session.has_feedback_pastes());
        assert_eq!(session.get_display_text_feedback(), "[Pasted]");
        assert_eq!(session.get_submit_text_feedback(), "feedback content");
    }

    #[test]
    fn test_clear_pastes() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("content".to_string());
        session.insert_paste_feedback("feedback".to_string());

        assert!(session.has_tab_input_pastes());
        assert!(session.has_feedback_pastes());

        session.clear_tab_input_pastes();
        session.clear_feedback_pastes();

        assert!(!session.has_tab_input_pastes());
        assert!(!session.has_feedback_pastes());
    }

    #[test]
    fn test_empty_paste_ignored() {
        let mut session = Session::new(0);
        session.insert_paste_tab_input("".to_string());

        assert!(!session.has_tab_input_pastes());
        assert!(session.tab_input.is_empty());
    }

    #[test]
    fn test_insert_feedback_newline() {
        let mut session = Session::new(0);
        session.user_feedback = "hello".to_string();
        session.cursor_position = 5;

        session.insert_feedback_newline();

        assert_eq!(session.user_feedback, "hello\n");
        assert_eq!(session.cursor_position, 6);
    }

    #[test]
    fn test_insert_feedback_newline_middle() {
        let mut session = Session::new(0);
        session.user_feedback = "hello world".to_string();
        session.cursor_position = 5;

        session.insert_feedback_newline();

        assert_eq!(session.user_feedback, "hello\n world");
        assert_eq!(session.cursor_position, 6);
    }

    // ===== Chat tabs tests =====

    #[test]
    fn test_add_run_tab() {
        let mut session = Session::new(0);
        assert!(session.run_tabs.is_empty());

        session.add_run_tab("Planning".to_string());
        assert_eq!(session.run_tabs.len(), 1);
        assert_eq!(session.run_tabs[0].phase, "Planning");
        assert_eq!(session.active_run_tab, 0);

        session.add_run_tab("Reviewing".to_string());
        assert_eq!(session.run_tabs.len(), 2);
        assert_eq!(session.active_run_tab, 1); // Auto-switched to newest
    }

    #[test]
    fn test_add_chat_message() {
        let mut session = Session::new(0);

        session.add_chat_message("claude", "Planning", "Hello world".to_string());
        assert_eq!(session.run_tabs.len(), 1);
        assert_eq!(session.run_tabs[0].messages.len(), 1);
        assert_eq!(session.run_tabs[0].messages[0].agent_name, "claude");
        assert_eq!(session.run_tabs[0].messages[0].message, "Hello world");
    }

    #[test]
    fn test_add_chat_message_creates_tab_if_needed() {
        let mut session = Session::new(0);

        // Adding a message creates the tab automatically
        session.add_chat_message("codex", "Reviewing", "Test message".to_string());
        assert_eq!(session.run_tabs.len(), 1);
        assert_eq!(session.run_tabs[0].phase, "Reviewing");
    }

    #[test]
    fn test_add_chat_message_filters_empty() {
        let mut session = Session::new(0);
        session.add_run_tab("Planning".to_string());

        session.add_chat_message("claude", "Planning", "".to_string());
        session.add_chat_message("claude", "Planning", "   ".to_string());
        assert!(session.run_tabs[0].messages.is_empty());
    }

    #[test]
    fn test_run_tab_navigation() {
        let mut session = Session::new(0);
        session.add_run_tab("Planning".to_string());
        session.add_run_tab("Reviewing".to_string());
        session.add_run_tab("Revising".to_string());

        assert_eq!(session.active_run_tab, 2);

        session.prev_run_tab();
        assert_eq!(session.active_run_tab, 1);

        session.prev_run_tab();
        assert_eq!(session.active_run_tab, 0);

        session.prev_run_tab(); // Should not go below 0
        assert_eq!(session.active_run_tab, 0);

        session.next_run_tab();
        assert_eq!(session.active_run_tab, 1);

        session.next_run_tab();
        assert_eq!(session.active_run_tab, 2);

        session.next_run_tab(); // Should not exceed max
        assert_eq!(session.active_run_tab, 2);
    }

    #[test]
    fn test_clear_chat_tabs() {
        let mut session = Session::new(0);
        session.add_run_tab("Planning".to_string());
        session.add_chat_message("claude", "Planning", "Test".to_string());
        session.chat_follow_mode = false;

        session.clear_chat_tabs();

        assert!(session.run_tabs.is_empty());
        assert_eq!(session.active_run_tab, 0);
        assert!(session.chat_follow_mode);
    }
}
