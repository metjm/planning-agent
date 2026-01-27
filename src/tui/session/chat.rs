use super::model::{ChatMessage, RunTab, RunTabEntry, SummaryState, ToolTimelineEntry};
use super::Session;

/// Normalize a phase name by stripping trailing " Summary" suffix.
/// This is a defensive measure to ensure summary agent output is routed
/// to the existing phase tab rather than creating a new tab.
fn normalize_phase(phase: &str) -> &str {
    phase.strip_suffix(" Summary").unwrap_or(phase)
}

impl Session {
    pub fn add_run_tab(&mut self, phase: String) {
        self.run_tabs.push(RunTab::new(phase));

        self.active_run_tab = self.run_tabs.len().saturating_sub(1);
    }

    pub fn add_chat_message(&mut self, agent_name: &str, phase: &str, message: String) {
        if message.trim().is_empty() {
            return;
        }

        let normalized_phase = normalize_phase(phase);
        let tab_idx = self
            .run_tabs
            .iter()
            .position(|t| t.phase == normalized_phase);
        let idx = match tab_idx {
            Some(i) => i,
            None => {
                self.add_run_tab(normalized_phase.to_string());
                self.run_tabs.len() - 1
            }
        };

        self.run_tabs[idx]
            .entries
            .push(RunTabEntry::Text(ChatMessage {
                agent_name: agent_name.to_string(),
                message,
            }));
    }

    pub fn add_tool_entry(&mut self, phase: &str, entry: ToolTimelineEntry) {
        let normalized_phase = normalize_phase(phase);
        let tab_idx = self
            .run_tabs
            .iter()
            .position(|t| t.phase == normalized_phase);
        let idx = match tab_idx {
            Some(i) => i,
            None => {
                self.add_run_tab(normalized_phase.to_string());
                self.run_tabs.len() - 1
            }
        };

        self.run_tabs[idx].entries.push(RunTabEntry::Tool(entry));
    }

    pub fn next_run_tab(&mut self) {
        if self.active_run_tab < self.run_tabs.len().saturating_sub(1) {
            self.active_run_tab += 1;
        }
    }

    pub fn prev_run_tab(&mut self) {
        self.active_run_tab = self.active_run_tab.saturating_sub(1);
    }

    pub fn chat_scroll_up(&mut self) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.scroll_position = tab.scroll_position.saturating_sub(1);
            self.chat_follow_mode = false;
        }
    }

    pub fn chat_scroll_down(&mut self, max_scroll: usize) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            if tab.scroll_position < max_scroll {
                tab.scroll_position = tab.scroll_position.saturating_add(1);
            }
        }
    }

    pub fn chat_scroll_to_bottom(&mut self) {
        self.chat_follow_mode = true;
    }

    pub fn summary_scroll_up(&mut self) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.summary_scroll = tab.summary_scroll.saturating_sub(1);
            tab.summary_follow_mode = false;
        }
    }

    pub fn summary_scroll_down(&mut self, max_scroll: usize) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            if tab.summary_scroll < max_scroll {
                tab.summary_scroll = tab.summary_scroll.saturating_add(1);
            }
        }
    }

    pub fn summary_scroll_to_top(&mut self) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.summary_scroll = 0;
            tab.summary_follow_mode = false;
        }
    }

    pub fn summary_scroll_to_bottom(&mut self, max_scroll: usize) {
        if let Some(tab) = self.run_tabs.get_mut(self.active_run_tab) {
            tab.summary_scroll = max_scroll;
            tab.summary_follow_mode = true;
        }
    }

    pub fn set_summary_generating(&mut self, phase: &str) {
        if let Some(tab) = self.run_tabs.iter_mut().find(|t| t.phase == phase) {
            tab.summary_state = SummaryState::Generating;
            tab.summary_spinner_frame = 0;
            tab.summary_text.clear();
        }
    }

    pub fn set_summary_ready(&mut self, phase: &str, summary: String) {
        if let Some(tab) = self.run_tabs.iter_mut().find(|t| t.phase == phase) {
            tab.summary_text = summary;
            tab.summary_state = SummaryState::Ready;
            tab.summary_scroll = 0;
            tab.summary_follow_mode = true;
        }
    }

    pub fn set_summary_error(&mut self, phase: &str, error: String) {
        if let Some(tab) = self.run_tabs.iter_mut().find(|t| t.phase == phase) {
            tab.summary_text = error;
            tab.summary_state = SummaryState::Error;
        }
    }

    pub fn advance_summary_spinners(&mut self) {
        for tab in &mut self.run_tabs {
            if tab.summary_state == SummaryState::Generating {
                tab.summary_spinner_frame = tab.summary_spinner_frame.wrapping_add(1);
            }
        }
    }
}
