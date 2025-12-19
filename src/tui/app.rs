use crate::state::{Phase, State};
use std::time::{Duration, Instant};

pub struct App {
    pub output_lines: Vec<String>,
    pub scroll_position: usize,
    pub workflow_state: Option<State>,
    pub start_time: Instant,
    pub total_cost: f64,
    pub running: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            output_lines: Vec::new(),
            scroll_position: 0,
            workflow_state: None,
            start_time: Instant::now(),
            total_cost: 0.0,
            running: true,
            should_quit: false,
        }
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
