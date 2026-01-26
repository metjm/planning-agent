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
}

/// All available slash commands for autocomplete.
pub const SLASH_COMMANDS: &[SlashCommandInfo] = &[
    SlashCommandInfo {
        command: "/update",
        description: "Install an available update",
    },
    SlashCommandInfo {
        command: "/config-dangerous",
        description: "Configure CLI tools to bypass approvals",
    },
    SlashCommandInfo {
        command: "/sessions",
        description: "View and resume workflow sessions",
    },
    SlashCommandInfo {
        command: "/max-iterations",
        description: "Set max iterations (e.g., /max-iterations 5)",
    },
    SlashCommandInfo {
        command: "/sequential",
        description: "Enable sequential review mode",
    },
    SlashCommandInfo {
        command: "/parallel",
        description: "Enable parallel review mode",
    },
    SlashCommandInfo {
        command: "/aggregation",
        description: "Set aggregation: any-rejects, all-reject, majority",
    },
    SlashCommandInfo {
        command: "/workflow",
        description: "Select workflow configuration",
    },
];

/// Commands that support dynamic argument completion.
/// Each entry is (command, max_parts) where max_parts limits how many
/// whitespace-separated tokens are valid (e.g., 2 means "command arg").
pub const COMMANDS_WITH_DYNAMIC_ARGS: &[(&str, usize)] = &[
    ("/config", 2),   // /config dangerous
    ("/workflow", 2), // /workflow <name>
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
    /// Cursor is in the argument position after a command that supports dynamic args
    DynamicArg {
        /// The command that triggered this context (e.g., "/config", "/workflow")
        command: String,
        /// Start of the whole command including the slash
        command_start: usize,
        /// End of the current token (cursor position)
        end_byte: usize,
        /// The argument text being typed (e.g., "dang", "claude")
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
        let query = text.get(..cursor_in_trimmed).unwrap_or("");
        return Some(SlashContext::Command {
            start_byte: trimmed_start,
            end_byte: cursor,
            query: query.to_string(),
        });
    }

    // Case 2: Check for commands with dynamic argument completion
    let first_token_lower = first_token.to_lowercase();
    for &(cmd, max_parts) in COMMANDS_WITH_DYNAMIC_ARGS {
        if first_token_lower == cmd && parts.len() <= max_parts {
            // Find where the second token starts (if any)
            let after_first = text.get(first_token_end..).unwrap_or("");
            let arg_text_start = after_first.len() - after_first.trim_start().len();
            let arg_start_in_text = first_token_end + arg_text_start;

            // The argument query is from arg_start_in_text to cursor_in_trimmed
            let arg_query = if cursor_in_trimmed > arg_start_in_text {
                text.get(arg_start_in_text..cursor_in_trimmed)
                    .unwrap_or("")
                    .to_string()
            } else {
                String::new()
            };

            // Only show autocomplete if we're actually in the argument position
            if cursor_in_trimmed > first_token_end {
                return Some(SlashContext::DynamicArg {
                    command: cmd.to_string(),
                    command_start: trimmed_start,
                    end_byte: cursor,
                    arg_query,
                });
            }
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
            let query_normalized: String =
                query_lower.chars().filter(|c| !c.is_whitespace()).collect();

            for cmd in SLASH_COMMANDS {
                let cmd_lower = cmd.command.to_lowercase();
                let cmd_normalized: String = cmd_lower.chars().filter(|c| *c != '-').collect();

                if let Some(score) =
                    compute_command_score(&query_normalized, &cmd_normalized, &cmd_lower)
                {
                    matches.push(SlashMatch {
                        display: cmd.command.to_string(),
                        insert: cmd.command.to_string(),
                        description: cmd.description.to_string(),
                        score,
                    });
                }
            }
        }
        SlashContext::DynamicArg {
            command, arg_query, ..
        } => {
            let arg_lower = arg_query.to_lowercase().trim().to_string();

            match command.as_str() {
                "/config" => {
                    // Provide "dangerous" as the only option
                    if arg_lower.is_empty() || "dangerous".starts_with(&arg_lower) {
                        let score = if arg_lower.is_empty() {
                            50
                        } else if "dangerous" == arg_lower {
                            100
                        } else {
                            80
                        };
                        matches.push(SlashMatch {
                            display: "/config dangerous".to_string(),
                            insert: "/config dangerous".to_string(),
                            description: "Configure CLI tools to bypass approvals".to_string(),
                            score,
                        });
                    }
                }
                "/workflow" => {
                    // Dynamically discover available workflows
                    let workflows =
                        crate::app::list_available_workflows_for_display().unwrap_or_default();

                    for wf in workflows {
                        let name_lower = wf.name.to_lowercase();
                        let score = if arg_lower.is_empty() {
                            50
                        } else if name_lower == arg_lower {
                            100
                        } else if name_lower.starts_with(&arg_lower) {
                            80
                        } else if name_lower.contains(&arg_lower) {
                            50
                        } else {
                            continue;
                        };

                        matches.push(SlashMatch {
                            display: format!("/workflow {}", wf.name),
                            insert: format!("/workflow {}", wf.name),
                            description: wf.source.clone(),
                            score,
                        });
                    }
                }
                _ => {}
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
fn compute_command_score(
    query_normalized: &str,
    cmd_normalized: &str,
    cmd_lower: &str,
) -> Option<i32> {
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
pub fn update_slash_state(slash_state: &mut SlashState, input: &str, cursor: usize) {
    match detect_slash_at_cursor(input, cursor) {
        Some(context) => {
            let (start, end) = match &context {
                SlashContext::Command {
                    start_byte,
                    end_byte,
                    ..
                } => (*start_byte, *end_byte),
                SlashContext::DynamicArg {
                    command_start,
                    end_byte,
                    ..
                } => (*command_start, *end_byte),
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
#[path = "tests/slash_tests.rs"]
mod tests;
