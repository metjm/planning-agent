//! Mention state for @-mention auto-complete in input fields.
use super::file_index::{FileIndex, MentionMatch};

/// Maximum number of matches to show in the dropdown
pub const MAX_MATCHES: usize = 10;

/// State tracking an active @-mention in an input field
#[derive(Debug, Clone, Default)]
pub struct MentionState {
    /// Whether a mention is currently active
    pub active: bool,
    /// The query string (text after the @)
    pub query: String,
    /// Byte position of the @ character in the input
    pub start_byte: usize,
    /// Current matches for the query
    pub matches: Vec<MentionMatch>,
    /// Currently selected match index
    pub selected_idx: usize,
}

impl MentionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the mention state
    pub fn clear(&mut self) {
        self.active = false;
        self.query.clear();
        self.start_byte = 0;
        self.matches.clear();
        self.selected_idx = 0;
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            if self.selected_idx == 0 {
                self.selected_idx = self.matches.len() - 1;
            } else {
                self.selected_idx -= 1;
            }
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected_idx = (self.selected_idx + 1) % self.matches.len();
        }
    }

    /// Get the currently selected match, if any
    pub fn selected_match(&self) -> Option<&MentionMatch> {
        if self.active && !self.matches.is_empty() {
            self.matches.get(self.selected_idx)
        } else {
            None
        }
    }
}

/// Detect if there's an active @-mention at the cursor position.
/// Returns Some((start_byte, query)) if a mention is active, None otherwise.
///
/// A mention is active when:
/// - There's an `@` character before the cursor
/// - The `@` is at the start of input or preceded by whitespace/punctuation
/// - There's no whitespace between the `@` and the cursor
/// - The `@` is not escaped (preceded by `\`)
pub fn detect_mention_at_cursor(input: &str, cursor: usize) -> Option<(usize, String)> {
    if cursor == 0 || cursor > input.len() {
        return None;
    }

    let text_before = &input[..cursor];

    // Find the nearest @ before cursor
    let mut search_pos = cursor;
    while search_pos > 0 {
        // Find @ going backwards
        let at_pos = text_before[..search_pos].rfind('@')?;

        // Check if this @ is escaped
        let is_escaped = at_pos > 0 && text_before.as_bytes().get(at_pos - 1) == Some(&b'\\');
        if is_escaped {
            // Try to find another @ before this one
            search_pos = at_pos;
            continue;
        }

        // Check if @ is at valid position (start or after whitespace/punctuation)
        let valid_position = at_pos == 0 || {
            let prev_char = text_before[..at_pos].chars().last().unwrap();
            prev_char.is_whitespace() || is_punctuation(prev_char)
        };

        if !valid_position {
            // Try to find another @ before this one
            search_pos = at_pos;
            continue;
        }

        // Extract the query (text between @ and cursor)
        let query_start = at_pos + 1;
        let query = &input[query_start..cursor];

        // Check for whitespace in the query - if found, mention is broken
        if query.chars().any(|c| c.is_whitespace()) {
            return None;
        }

        return Some((at_pos, query.to_string()));
    }

    None
}

fn is_punctuation(c: char) -> bool {
    matches!(
        c,
        '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '"' | '\'' | '`' | '<' | '>' | '/'
            | '\\' | '|' | '!' | '?' | '.' | '-' | '_' | '=' | '+' | '*' | '&' | '^' | '%' | '$'
            | '#' | '~'
    )
}

/// Update the mention state based on current input and cursor position.
pub fn update_mention_state(
    mention_state: &mut MentionState,
    input: &str,
    cursor: usize,
    file_index: &FileIndex,
) {
    match detect_mention_at_cursor(input, cursor) {
        Some((start_byte, query)) => {
            mention_state.active = true;
            mention_state.start_byte = start_byte;

            // Only update matches if query changed
            if mention_state.query != query {
                mention_state.query = query.clone();
                mention_state.matches = file_index.find_matches(&query, MAX_MATCHES);
                mention_state.selected_idx = 0;
            }
        }
        None => {
            mention_state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mention_at_start() {
        let result = detect_mention_at_cursor("@test", 5);
        assert_eq!(result, Some((0, "test".to_string())));
    }

    #[test]
    fn test_detect_mention_after_space() {
        let result = detect_mention_at_cursor("hello @wor", 10);
        assert_eq!(result, Some((6, "wor".to_string())));
    }

    #[test]
    fn test_detect_mention_after_newline() {
        let result = detect_mention_at_cursor("line1\n@file", 11);
        assert_eq!(result, Some((6, "file".to_string())));
    }

    #[test]
    fn test_detect_mention_after_punctuation() {
        let result = detect_mention_at_cursor("(@path", 6);
        assert_eq!(result, Some((1, "path".to_string())));
    }

    #[test]
    fn test_no_mention_without_at() {
        let result = detect_mention_at_cursor("hello world", 11);
        assert_eq!(result, None);
    }

    #[test]
    fn test_no_mention_with_space_in_query() {
        let result = detect_mention_at_cursor("@hello world", 12);
        assert_eq!(result, None);
    }

    #[test]
    fn test_escaped_at_ignored() {
        let result = detect_mention_at_cursor("\\@test", 6);
        assert_eq!(result, None);
    }

    #[test]
    fn test_at_in_middle_of_word() {
        let result = detect_mention_at_cursor("email@test", 10);
        assert_eq!(result, None);
    }

    #[test]
    fn test_empty_query() {
        let result = detect_mention_at_cursor("@", 1);
        assert_eq!(result, Some((0, "".to_string())));
    }

    #[test]
    fn test_cursor_at_start() {
        let result = detect_mention_at_cursor("@test", 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_mention_state_select_prev() {
        let mut state = MentionState {
            active: true,
            query: "test".to_string(),
            start_byte: 0,
            matches: vec![
                MentionMatch { path: "a".to_string(), score: 10 },
                MentionMatch { path: "b".to_string(), score: 9 },
                MentionMatch { path: "c".to_string(), score: 8 },
            ],
            selected_idx: 0,
        };

        state.select_prev();
        assert_eq!(state.selected_idx, 2); // Wraps to end

        state.select_prev();
        assert_eq!(state.selected_idx, 1);
    }

    #[test]
    fn test_mention_state_select_next() {
        let mut state = MentionState {
            active: true,
            query: "test".to_string(),
            start_byte: 0,
            matches: vec![
                MentionMatch { path: "a".to_string(), score: 10 },
                MentionMatch { path: "b".to_string(), score: 9 },
            ],
            selected_idx: 0,
        };

        state.select_next();
        assert_eq!(state.selected_idx, 1);

        state.select_next();
        assert_eq!(state.selected_idx, 0); // Wraps to start
    }

    #[test]
    fn test_mention_state_clear() {
        let mut state = MentionState {
            active: true,
            query: "test".to_string(),
            start_byte: 5,
            matches: vec![MentionMatch { path: "a".to_string(), score: 10 }],
            selected_idx: 0,
        };

        state.clear();
        assert!(!state.active);
        assert!(state.query.is_empty());
        assert_eq!(state.start_byte, 0);
        assert!(state.matches.is_empty());
        assert_eq!(state.selected_idx, 0);
    }

    #[test]
    fn test_selected_match() {
        let mut state = MentionState {
            active: true,
            query: "test".to_string(),
            start_byte: 0,
            matches: vec![
                MentionMatch { path: "a".to_string(), score: 10 },
                MentionMatch { path: "b".to_string(), score: 9 },
            ],
            selected_idx: 1,
        };

        assert_eq!(state.selected_match().unwrap().path, "b");

        state.active = false;
        assert!(state.selected_match().is_none());
    }
}
