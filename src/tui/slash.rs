//! Slash command autocomplete state and matching for the NamingTab input.
//!
//! Provides discovery and completion for slash commands like `/update` and
//! `/config-dangerous` (or `/config dangerous`).

/// Maximum number of matches to show in the dropdown
pub const MAX_MATCHES: usize = 10;

/// Canonical definition of a slash command for autocomplete purposes.
#[derive(Debug, Clone)]
pub struct SlashCommandInfo {
    /// The primary command (e.g., "/update", "/config-dangerous")
    pub command: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Alternative input forms (e.g., "/config dangerous" for "/config-dangerous")
    /// Reserved for future use with fuzzy matching of aliases.
    #[allow(dead_code)]
    pub aliases: &'static [&'static str],
}

/// All available slash commands for autocomplete.
pub const SLASH_COMMANDS: &[SlashCommandInfo] = &[
    SlashCommandInfo {
        command: "/update",
        description: "Install an available update",
        aliases: &[],
    },
    SlashCommandInfo {
        command: "/config-dangerous",
        description: "Configure CLI tools to bypass approvals",
        aliases: &["/config dangerous"],
    },
];

/// A match result for slash command autocomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashMatch {
    /// The text to display in the dropdown (e.g., "/update")
    pub display: String,
    /// The text to insert when accepted
    pub insert: String,
    /// Short description of the command
    pub description: String,
    /// Match score (higher is better)
    pub score: i32,
}

/// State tracking an active slash command autocomplete.
#[derive(Debug, Clone, Default)]
pub struct SlashState {
    /// Whether slash autocomplete is currently active
    pub active: bool,
    /// Byte position where the slash token starts
    pub start_byte: usize,
    /// Byte position where the current token ends (cursor position)
    pub end_byte: usize,
    /// Current matches for the query
    pub matches: Vec<SlashMatch>,
    /// Currently selected match index
    pub selected_idx: usize,
}

impl SlashState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the slash state
    pub fn clear(&mut self) {
        self.active = false;
        self.start_byte = 0;
        self.end_byte = 0;
        self.matches.clear();
        self.selected_idx = 0;
    }

    /// Move selection up (wraps around)
    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            if self.selected_idx == 0 {
                self.selected_idx = self.matches.len() - 1;
            } else {
                self.selected_idx -= 1;
            }
        }
    }

    /// Move selection down (wraps around)
    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected_idx = (self.selected_idx + 1) % self.matches.len();
        }
    }

    /// Get the currently selected match, if any
    pub fn selected_match(&self) -> Option<&SlashMatch> {
        if self.active && !self.matches.is_empty() {
            self.matches.get(self.selected_idx)
        } else {
            None
        }
    }
}

/// Represents the detected slash command context at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashContext {
    /// Cursor is within the main slash command token (e.g., `/upd|ate`)
    Command {
        start_byte: usize,
        end_byte: usize,
        /// The partial command text including the leading `/`
        query: String,
    },
    /// Cursor is in the argument position after `/config ` (for `/config dangerous`)
    ConfigArg {
        /// Start of the whole command including `/config`
        command_start: usize,
        /// End of the current token (cursor position)
        end_byte: usize,
        /// The argument text being typed (e.g., "dang")
        arg_query: String,
    },
}

/// Detect if there's a slash command context at the cursor position.
///
/// A slash command is only valid when:
/// - The `/` is at the start of the input (after optional whitespace)
/// - There's no whitespace between the `/` and the cursor (for command matching)
/// - Or we're in the argument position for `/config dangerous`
pub fn detect_slash_at_cursor(input: &str, cursor: usize) -> Option<SlashContext> {
    if cursor == 0 || cursor > input.len() {
        return None;
    }

    let trimmed_start = input.len() - input.trim_start().len();
    let text = input.trim_start();

    // Input must start with `/`
    if !text.starts_with('/') {
        return None;
    }

    // Find the command token boundaries
    let cursor_in_trimmed = if cursor > trimmed_start {
        cursor - trimmed_start
    } else {
        return None;
    };

    // Split the trimmed text to analyze tokens
    let parts: Vec<&str> = text.split_whitespace().collect();

    if parts.is_empty() {
        return None;
    }

    let first_token = parts[0];
    let first_token_end = text.find(first_token).unwrap_or(0) + first_token.len();

    // Case 1: Cursor is within the first token (the command itself)
    if cursor_in_trimmed <= first_token_end {
        let query = &text[..cursor_in_trimmed];
        return Some(SlashContext::Command {
            start_byte: trimmed_start,
            end_byte: cursor,
            query: query.to_string(),
        });
    }

    // Case 2: Check for `/config` with argument
    let first_token_lower = first_token.to_lowercase();
    if first_token_lower == "/config" && parts.len() <= 2 {
        // Find where the second token starts (if any)
        let after_first = &text[first_token_end..];
        let arg_text_start = after_first.len() - after_first.trim_start().len();
        let arg_start_in_text = first_token_end + arg_text_start;

        // The argument query is from arg_start_in_text to cursor_in_trimmed
        let arg_query = if cursor_in_trimmed > arg_start_in_text {
            text[arg_start_in_text..cursor_in_trimmed].to_string()
        } else {
            String::new()
        };

        // Only show autocomplete if we're actually in the argument position
        // (i.e., there's a space after /config)
        if cursor_in_trimmed > first_token_end {
            return Some(SlashContext::ConfigArg {
                command_start: trimmed_start,
                end_byte: cursor,
                arg_query,
            });
        }
    }

    None
}

/// Find matching slash commands for the given context.
pub fn find_slash_matches(context: &SlashContext, limit: usize) -> Vec<SlashMatch> {
    let mut matches: Vec<SlashMatch> = Vec::new();

    match context {
        SlashContext::Command { query, .. } => {
            let query_lower = query.to_lowercase();
            // Normalize query: treat hyphens and spaces equivalently
            let query_normalized: String = query_lower.chars().filter(|c| !c.is_whitespace()).collect();

            for cmd in SLASH_COMMANDS {
                let cmd_lower = cmd.command.to_lowercase();
                let cmd_normalized: String = cmd_lower.chars().filter(|c| *c != '-').collect();

                if let Some(score) = compute_command_score(&query_normalized, &cmd_normalized, &cmd_lower) {
                    matches.push(SlashMatch {
                        display: cmd.command.to_string(),
                        insert: cmd.command.to_string(),
                        description: cmd.description.to_string(),
                        score,
                    });
                }
            }
        }
        SlashContext::ConfigArg { arg_query, .. } => {
            // Only suggest "dangerous" when typing after `/config `
            let arg_lower = arg_query.to_lowercase().trim().to_string();

            // Check if "dangerous" matches the query
            if arg_lower.is_empty() || "dangerous".starts_with(&arg_lower) {
                let score = if arg_lower.is_empty() {
                    50 // Default suggestion
                } else if "dangerous" == arg_lower {
                    100 // Exact match
                } else if "dangerous".starts_with(&arg_lower) {
                    80 // Prefix match
                } else {
                    0
                };

                if score > 0 {
                    matches.push(SlashMatch {
                        display: "/config dangerous".to_string(),
                        insert: "/config dangerous".to_string(),
                        description: "Configure CLI tools to bypass approvals".to_string(),
                        score,
                    });
                }
            }
        }
    }

    // Sort by score (descending), then by command name (ascending)
    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.display.cmp(&b.display))
    });

    matches.truncate(limit);
    matches
}

/// Compute a score for how well a query matches a command.
fn compute_command_score(query_normalized: &str, cmd_normalized: &str, cmd_lower: &str) -> Option<i32> {
    // Must start with /
    if !query_normalized.starts_with('/') {
        return None;
    }

    // Exact match
    if cmd_lower == query_normalized.replace('-', "") {
        return Some(100);
    }

    // Prefix match
    if cmd_normalized.starts_with(query_normalized) {
        return Some(80);
    }

    // Substring match
    if cmd_normalized.contains(query_normalized) {
        return Some(50);
    }

    None
}

/// Update the slash state based on current input and cursor position.
pub fn update_slash_state(
    slash_state: &mut SlashState,
    input: &str,
    cursor: usize,
) {
    match detect_slash_at_cursor(input, cursor) {
        Some(context) => {
            let (start, end) = match &context {
                SlashContext::Command { start_byte, end_byte, .. } => (*start_byte, *end_byte),
                SlashContext::ConfigArg { command_start, end_byte, .. } => (*command_start, *end_byte),
            };

            let matches = find_slash_matches(&context, MAX_MATCHES);

            slash_state.active = !matches.is_empty();
            slash_state.start_byte = start;
            slash_state.end_byte = end;
            slash_state.matches = matches;
            // Reset selection when matches change
            slash_state.selected_idx = 0;
        }
        None => {
            slash_state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_slash_at_start() {
        let result = detect_slash_at_cursor("/u", 2);
        assert!(matches!(
            result,
            Some(SlashContext::Command { query, .. }) if query == "/u"
        ));
    }

    #[test]
    fn test_detect_slash_with_leading_whitespace() {
        let result = detect_slash_at_cursor("  /upd", 6);
        assert!(matches!(
            result,
            Some(SlashContext::Command { query, start_byte, .. }) if query == "/upd" && start_byte == 2
        ));
    }

    #[test]
    fn test_detect_slash_empty_after_slash() {
        let result = detect_slash_at_cursor("/", 1);
        assert!(matches!(
            result,
            Some(SlashContext::Command { query, .. }) if query == "/"
        ));
    }

    #[test]
    fn test_detect_config_arg() {
        let result = detect_slash_at_cursor("/config d", 9);
        assert!(matches!(
            result,
            Some(SlashContext::ConfigArg { arg_query, .. }) if arg_query == "d"
        ));
    }

    #[test]
    fn test_detect_config_arg_empty() {
        let result = detect_slash_at_cursor("/config ", 8);
        assert!(matches!(
            result,
            Some(SlashContext::ConfigArg { arg_query, .. }) if arg_query.is_empty()
        ));
    }

    #[test]
    fn test_detect_no_slash() {
        let result = detect_slash_at_cursor("hello", 5);
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_non_leading_slash() {
        let result = detect_slash_at_cursor("hello /update", 13);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_matches_update() {
        let context = SlashContext::Command {
            start_byte: 0,
            end_byte: 2,
            query: "/u".to_string(),
        };
        let matches = find_slash_matches(&context, 10);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.display == "/update"));
    }

    #[test]
    fn test_find_matches_config() {
        let context = SlashContext::Command {
            start_byte: 0,
            end_byte: 4,
            query: "/con".to_string(),
        };
        let matches = find_slash_matches(&context, 10);
        assert!(!matches.is_empty());
        assert!(matches.iter().any(|m| m.display == "/config-dangerous"));
    }

    #[test]
    fn test_find_matches_config_arg() {
        let context = SlashContext::ConfigArg {
            command_start: 0,
            end_byte: 9,
            arg_query: "d".to_string(),
        };
        let matches = find_slash_matches(&context, 10);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].display, "/config dangerous");
    }

    #[test]
    fn test_exact_match_scores_highest() {
        let context = SlashContext::Command {
            start_byte: 0,
            end_byte: 7,
            query: "/update".to_string(),
        };
        let matches = find_slash_matches(&context, 10);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].display, "/update");
        assert_eq!(matches[0].score, 100);
    }

    #[test]
    fn test_slash_state_navigation() {
        let mut state = SlashState {
            active: true,
            start_byte: 0,
            end_byte: 2,
            matches: vec![
                SlashMatch {
                    display: "/update".to_string(),
                    insert: "/update".to_string(),
                    description: "desc".to_string(),
                    score: 100,
                },
                SlashMatch {
                    display: "/config-dangerous".to_string(),
                    insert: "/config-dangerous".to_string(),
                    description: "desc".to_string(),
                    score: 50,
                },
            ],
            selected_idx: 0,
        };

        state.select_next();
        assert_eq!(state.selected_idx, 1);

        state.select_next();
        assert_eq!(state.selected_idx, 0); // Wraps

        state.select_prev();
        assert_eq!(state.selected_idx, 1); // Wraps backwards
    }

    #[test]
    fn test_update_slash_state() {
        let mut state = SlashState::new();
        update_slash_state(&mut state, "/up", 3);
        assert!(state.active);
        assert!(!state.matches.is_empty());
        assert_eq!(state.start_byte, 0);
        assert_eq!(state.end_byte, 3);

        // Clear when not a slash command
        update_slash_state(&mut state, "hello", 5);
        assert!(!state.active);
        assert!(state.matches.is_empty());
    }

    #[test]
    fn test_slash_state_clear() {
        let mut state = SlashState {
            active: true,
            start_byte: 0,
            end_byte: 5,
            matches: vec![SlashMatch {
                display: "test".to_string(),
                insert: "test".to_string(),
                description: "desc".to_string(),
                score: 100,
            }],
            selected_idx: 0,
        };

        state.clear();
        assert!(!state.active);
        assert_eq!(state.start_byte, 0);
        assert_eq!(state.end_byte, 0);
        assert!(state.matches.is_empty());
        assert_eq!(state.selected_idx, 0);
    }

    #[test]
    fn test_selected_match() {
        let mut state = SlashState {
            active: true,
            start_byte: 0,
            end_byte: 2,
            matches: vec![
                SlashMatch {
                    display: "/update".to_string(),
                    insert: "/update".to_string(),
                    description: "desc".to_string(),
                    score: 100,
                },
                SlashMatch {
                    display: "/config-dangerous".to_string(),
                    insert: "/config-dangerous".to_string(),
                    description: "desc".to_string(),
                    score: 50,
                },
            ],
            selected_idx: 1,
        };

        assert_eq!(state.selected_match().unwrap().display, "/config-dangerous");

        state.active = false;
        assert!(state.selected_match().is_none());
    }
}
