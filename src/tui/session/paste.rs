use super::model::PasteBlock;
use super::Session;

impl Session {
    pub fn insert_paste_tab_input(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = normalized.lines().count().max(1);

        let paste_block = PasteBlock {
            content: text,
            start_pos: self.tab_input_cursor,
            line_count,
        };

        let placeholder = Self::format_paste_placeholder(line_count);
        self.tab_input
            .insert_str(self.tab_input_cursor, &placeholder);
        self.tab_input_cursor += placeholder.len();

        for paste in &mut self.tab_input_pastes {
            if paste.start_pos >= paste_block.start_pos {
                paste.start_pos += placeholder.len();
            }
        }

        self.tab_input_pastes.push(paste_block);
    }

    pub fn insert_paste_feedback(&mut self, text: String) {
        if text.is_empty() {
            return;
        }

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = normalized.lines().count().max(1);

        let paste_block = PasteBlock {
            content: text,
            start_pos: self.cursor_position,
            line_count,
        };

        let placeholder = Self::format_paste_placeholder(line_count);
        self.user_feedback
            .insert_str(self.cursor_position, &placeholder);
        self.cursor_position += placeholder.len();

        for paste in &mut self.feedback_pastes {
            if paste.start_pos >= paste_block.start_pos {
                paste.start_pos += placeholder.len();
            }
        }

        self.feedback_pastes.push(paste_block);
    }

    pub(crate) fn format_paste_placeholder(line_count: usize) -> String {
        if line_count <= 1 {
            "[Pasted]".to_string()
        } else {
            format!("[Pasted +{} lines]", line_count - 1)
        }
    }

    pub fn delete_paste_at_cursor_tab(&mut self) -> bool {
        if let Some(idx) = self.find_paste_at_cursor_tab() {
            let paste = self.tab_input_pastes.remove(idx);
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_len = placeholder.len();

            self.tab_input = format!(
                "{}{}",
                self.tab_input.get(..paste.start_pos).unwrap_or(""),
                self.tab_input
                    .get(paste.start_pos + placeholder_len..)
                    .unwrap_or("")
            );

            self.tab_input_cursor = paste.start_pos;

            for p in &mut self.tab_input_pastes {
                if p.start_pos > paste.start_pos {
                    p.start_pos -= placeholder_len;
                }
            }

            return true;
        }
        false
    }

    fn find_paste_at_cursor_tab(&self) -> Option<usize> {
        for (idx, paste) in self.tab_input_pastes.iter().enumerate() {
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_end = paste.start_pos + placeholder.len();

            if self.tab_input_cursor > paste.start_pos && self.tab_input_cursor <= placeholder_end {
                return Some(idx);
            }
        }
        None
    }

    pub fn get_display_text_tab(&self) -> String {
        self.tab_input.clone()
    }

    pub fn get_display_text_feedback(&self) -> String {
        self.user_feedback.clone()
    }

    pub fn get_submit_text_tab(&self) -> String {
        self.expand_pastes_in_text(&self.tab_input, &self.tab_input_pastes)
    }

    pub fn get_submit_text_feedback(&self) -> String {
        self.expand_pastes_in_text(&self.user_feedback, &self.feedback_pastes)
    }

    fn expand_pastes_in_text(&self, text: &str, pastes: &[PasteBlock]) -> String {
        if pastes.is_empty() {
            return text.to_string();
        }

        let mut sorted_pastes: Vec<_> = pastes.iter().collect();
        sorted_pastes.sort_by(|a, b| b.start_pos.cmp(&a.start_pos));

        let mut result = text.to_string();

        for paste in sorted_pastes {
            let placeholder = Self::format_paste_placeholder(paste.line_count);
            let placeholder_end = paste.start_pos + placeholder.len();

            if placeholder_end <= result.len() {
                result = format!(
                    "{}{}{}",
                    result.get(..paste.start_pos).unwrap_or(""),
                    &paste.content,
                    result.get(placeholder_end..).unwrap_or("")
                );
            }
        }

        result
    }

    pub fn clear_tab_input_pastes(&mut self) {
        self.tab_input_pastes.clear();
    }

    pub fn clear_feedback_pastes(&mut self) {
        self.feedback_pastes.clear();
    }

    pub fn has_tab_input_pastes(&self) -> bool {
        !self.tab_input_pastes.is_empty()
    }

    pub fn has_feedback_pastes(&self) -> bool {
        !self.feedback_pastes.is_empty()
    }
}
