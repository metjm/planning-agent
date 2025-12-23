
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::Session;

impl Session {

    pub fn insert_tab_input_char(&mut self, c: char) {
        self.tab_input.insert(self.tab_input_cursor, c);
        self.tab_input_cursor += c.len_utf8();
    }

    pub fn delete_tab_input_char(&mut self) {
        if self.tab_input_cursor > 0 {

            let prev_char_boundary = self.tab_input[..self.tab_input_cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.tab_input.remove(prev_char_boundary);
            self.tab_input_cursor = prev_char_boundary;
        }
    }

    pub fn move_tab_input_cursor_left(&mut self) {
        if self.tab_input_cursor > 0 {

            self.tab_input_cursor = self.tab_input[..self.tab_input_cursor]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    pub fn move_tab_input_cursor_right(&mut self) {
        if self.tab_input_cursor < self.tab_input.len() {

            if let Some((_, c)) = self.tab_input[self.tab_input_cursor..].char_indices().next() {
                self.tab_input_cursor += c.len_utf8();
            }
        }
    }

    pub fn insert_tab_input_newline(&mut self) {
        self.tab_input.insert(self.tab_input_cursor, '\n');
        self.tab_input_cursor += '\n'.len_utf8();
    }

    pub fn move_tab_input_cursor_up(&mut self) {
        let text_before = &self.tab_input[..self.tab_input_cursor];

        let current_line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);

        if current_line_start == 0 {
            return;
        }

        let display_col = self.tab_input[current_line_start..self.tab_input_cursor].width();

        let prev_line_end = current_line_start - 1; 
        let prev_line_start = self.tab_input[..prev_line_end]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);

        let prev_line = &self.tab_input[prev_line_start..prev_line_end];
        let mut accumulated_width = 0;
        let mut target_byte_offset = prev_line.len(); 
        for (idx, c) in prev_line.char_indices() {
            if accumulated_width >= display_col {
                target_byte_offset = idx;
                break;
            }
            accumulated_width += c.width().unwrap_or(0);
        }

        self.tab_input_cursor = prev_line_start + target_byte_offset;
    }

    pub fn move_tab_input_cursor_down(&mut self) {
        let text_before = &self.tab_input[..self.tab_input_cursor];
        let text_after = &self.tab_input[self.tab_input_cursor..];

        let current_line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);

        let display_col = self.tab_input[current_line_start..self.tab_input_cursor].width();

        let next_newline = text_after.find('\n');

        let Some(offset) = next_newline else {
            return;
        };

        let next_line_start = self.tab_input_cursor + offset + 1;

        let next_line_end = self.tab_input[next_line_start..]
            .find('\n')
            .map(|p| next_line_start + p)
            .unwrap_or(self.tab_input.len());

        let next_line = &self.tab_input[next_line_start..next_line_end];
        let mut accumulated_width = 0;
        let mut target_byte_offset = next_line.len(); 
        for (idx, c) in next_line.char_indices() {
            if accumulated_width >= display_col {
                target_byte_offset = idx;
                break;
            }
            accumulated_width += c.width().unwrap_or(0);
        }

        self.tab_input_cursor = next_line_start + target_byte_offset;
    }

    pub fn get_tab_input_cursor_position(&self) -> (usize, usize) {
        let text_before = &self.tab_input[..self.tab_input_cursor];
        let line = text_before.matches('\n').count();
        let line_start = text_before.rfind('\n').map(|p| p + 1).unwrap_or(0);

        let col = self.tab_input[line_start..self.tab_input_cursor].width();
        (line, col)
    }

    pub fn get_tab_input_line_count(&self) -> usize {
        self.tab_input.matches('\n').count() + 1
    }

    pub fn insert_char(&mut self, c: char) {
        self.user_feedback.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {

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

            self.cursor_position = self.user_feedback[..self.cursor_position]
                .char_indices()
                .last()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.user_feedback.len() {

            if let Some((_, c)) = self.user_feedback[self.cursor_position..].char_indices().next() {
                self.cursor_position += c.len_utf8();
            }
        }
    }

    pub fn insert_feedback_newline(&mut self) {
        self.user_feedback.insert(self.cursor_position, '\n');
        self.cursor_position += '\n'.len_utf8();
    }

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
}
