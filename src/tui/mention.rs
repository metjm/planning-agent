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

    let text_before = input.get(..cursor)?;

    // Find the nearest @ before cursor
    let mut search_pos = cursor;
    while search_pos > 0 {
        // Find @ going backwards
        let at_pos = text_before.get(..search_pos)?.rfind('@')?;

        // Check if this @ is escaped
        let is_escaped = at_pos > 0 && text_before.as_bytes().get(at_pos - 1) == Some(&b'\\');
        if is_escaped {
            // Try to find another @ before this one
            search_pos = at_pos;
            continue;
        }

        // Check if @ is at valid position (start or after whitespace/punctuation)
        let valid_position = at_pos == 0 || {
            let prev_char = text_before.get(..at_pos)?.chars().last()?;
            prev_char.is_whitespace() || is_punctuation(prev_char)
        };

        if !valid_position {
            // Try to find another @ before this one
            search_pos = at_pos;
            continue;
        }

        // Extract the query (text between @ and cursor)
        let query_start = at_pos + 1;
        let query = input.get(query_start..cursor)?;

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
        '(' | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | ','
            | ';'
            | ':'
            | '"'
            | '\''
            | '`'
            | '<'
            | '>'
            | '/'
            | '\\'
            | '|'
            | '!'
            | '?'
            | '.'
            | '-'
            | '_'
            | '='
            | '+'
            | '*'
            | '&'
            | '^'
            | '%'
            | '$'
            | '#'
            | '~'
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
#[path = "tests/mention_tests.rs"]
mod tests;
