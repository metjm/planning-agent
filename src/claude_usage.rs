use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct ClaudeUsage {
    /// Weekly usage remaining as percentage (e.g., 45 means 45% remaining)
    pub weekly_remaining: Option<u8>,
    /// Session/daily usage remaining as percentage
    pub session_remaining: Option<u8>,
    /// User's plan type (e.g., "Max", "Pro", "Free")
    pub plan_type: Option<String>,
    /// When this data was fetched
    pub fetched_at: Option<Instant>,
    /// Error message if fetch failed
    pub error_message: Option<String>,
}

impl ClaudeUsage {
    /// Check if the usage data is stale (older than 5 minutes)
    /// Reserved for future use (manual refresh keybind)
    #[allow(dead_code)]
    pub fn is_stale(&self) -> bool {
        match self.fetched_at {
            Some(t) => t.elapsed() > Duration::from_secs(300), // 5 minutes
            None => true,
        }
    }

    pub fn claude_not_available() -> Self {
        Self {
            error_message: Some("Claude CLI not found".to_string()),
            ..Default::default()
        }
    }

    fn with_error(msg: String) -> Self {
        Self {
            error_message: Some(msg),
            ..Default::default()
        }
    }
}

/// Check if the `claude` CLI is available using the `which` crate (cross-platform)
pub fn is_claude_available() -> bool {
    which::which("claude").is_ok()
}

/// Run Claude CLI in a PTY and execute /usage command
fn run_claude_usage_via_pty(command: &str, timeout: Duration) -> Result<String, String> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to allocate PTY: {}", e))?;

    let cmd = CommandBuilder::new(command);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn Claude: {}", e))?;

    // Drop the slave to avoid blocking
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to get PTY reader: {}", e))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

    // Spawn reader thread to collect output
    let (tx, rx) = mpsc::channel();
    let reader_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut reader = reader;
        let mut chunk = [0u8; 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        tx.send(buf).ok();
    });

    let start = Instant::now();

    // Wait for prompt (fixed delay, don't parse for ">")
    std::thread::sleep(Duration::from_secs(3));

    // Check if we've already timed out
    if start.elapsed() > timeout {
        let _ = child.kill();
        return Err("Timeout waiting for Claude CLI".to_string());
    }

    // Send /usage command
    writer
        .write_all(b"/usage\r")
        .map_err(|e| format!("Failed to send /usage: {}", e))?;

    // Wait for output
    std::thread::sleep(Duration::from_secs(2));

    // Request graceful exit
    let _ = writer.write_all(b"/exit\r");

    // Wait for child to exit, with timeout
    let exit_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                let _ = child.kill();
                break;
            }
        }
    }

    // Drop writer to signal EOF to reader
    drop(writer);

    // Join reader thread with timeout
    let output = rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    let _ = reader_handle.join();

    // Drop the master to clean up PTY
    drop(pair.master);

    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Strip ANSI escape sequences from text
fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Fetch Claude usage by running /usage command via PTY
pub fn fetch_claude_usage_sync() -> ClaudeUsage {
    if !is_claude_available() {
        return ClaudeUsage::claude_not_available();
    }

    let timeout = Duration::from_secs(12);

    match run_claude_usage_via_pty("claude", timeout) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);

            // Parse usage percentages and plan info
            let weekly = parse_usage_percent(&output, "week");
            let session = parse_usage_percent(&output, "session")
                .or_else(|| parse_usage_percent(&output, "daily"));
            let plan = parse_plan_type(&output);

            ClaudeUsage {
                weekly_remaining: weekly,
                session_remaining: session,
                plan_type: plan,
                fetched_at: Some(Instant::now()),
                error_message: None,
            }
        }
        Err(e) => ClaudeUsage::with_error(e),
    }
}

/// Parse percentage from text, looking for patterns like "80%" near a keyword
fn parse_usage_percent(text: &str, keyword: &str) -> Option<u8> {
    for line in text.lines() {
        let line_lower = line.to_lowercase();
        if line_lower.contains(keyword) {
            if let Some(pos) = line.find('%') {
                let before = &line[..pos];
                let digits: String = before
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                if !digits.is_empty() {
                    return digits.parse().ok();
                }
            }
        }
    }
    None
}

/// Parse plan type from output (e.g., "Plan: Max" -> "Max")
fn parse_plan_type(text: &str) -> Option<String> {
    for line in text.lines() {
        let line_lower = line.to_lowercase();
        if line_lower.contains("plan") {
            // Look for pattern like "Plan: Max" or "plan: pro"
            if let Some(colon_pos) = line.find(':') {
                let after_colon = line[colon_pos + 1..].trim();
                if !after_colon.is_empty() {
                    // Take first word after colon
                    let plan = after_colon.split_whitespace().next()?;
                    return Some(plan.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_usage_percent() {
        assert_eq!(parse_usage_percent("Weekly: 80%", "week"), Some(80));
        assert_eq!(
            parse_usage_percent("Session usage: 25%", "session"),
            Some(25)
        );
        assert_eq!(parse_usage_percent("Daily usage: 100%", "daily"), Some(100));
        assert_eq!(parse_usage_percent("No percentage here", "week"), None);
    }

    #[test]
    fn test_parse_plan_type() {
        assert_eq!(parse_plan_type("Plan: Max"), Some("Max".to_string()));
        assert_eq!(
            parse_plan_type("Your plan: Pro tier"),
            Some("Pro".to_string())
        );
        assert_eq!(parse_plan_type("No plan info"), None);
    }

    #[test]
    fn test_claude_usage_is_stale() {
        let usage = ClaudeUsage::default();
        assert!(usage.is_stale());

        let fresh = ClaudeUsage {
            fetched_at: Some(Instant::now()),
            ..Default::default()
        };
        assert!(!fresh.is_stale());
    }

    #[test]
    fn test_strip_ansi_codes() {
        // Basic ANSI color codes
        assert_eq!(strip_ansi_codes("\x1b[32mHello\x1b[0m"), "Hello");
        assert_eq!(
            strip_ansi_codes("\x1b[1;31mBold Red\x1b[0m Text"),
            "Bold Red Text"
        );

        // No ANSI codes
        assert_eq!(strip_ansi_codes("Plain text"), "Plain text");

        // Multiple sequences
        assert_eq!(
            strip_ansi_codes("\x1b[33mYellow\x1b[0m \x1b[34mBlue\x1b[0m"),
            "Yellow Blue"
        );

        // Complex sequences with cursor movements
        assert_eq!(strip_ansi_codes("\x1b[2K\x1b[1GLine"), "Line");
    }

    #[test]
    fn test_parse_usage_with_ansi_codes() {
        // Test that ANSI stripping works with parsing
        let raw = "\x1b[32mWeekly usage: 80%\x1b[0m remaining";
        let stripped = strip_ansi_codes(raw);
        assert_eq!(parse_usage_percent(&stripped, "week"), Some(80));
    }

    #[test]
    fn test_parse_plan_with_ansi_codes() {
        let raw = "\x1b[1mPlan:\x1b[0m Max (premium)";
        let stripped = strip_ansi_codes(raw);
        assert_eq!(parse_plan_type(&stripped), Some("Max".to_string()));
    }

    #[test]
    fn test_no_expect_in_error_messages() {
        // Regression test: ensure no error messages mention "expect"
        let error_usage = ClaudeUsage::with_error("Some error".to_string());
        if let Some(msg) = &error_usage.error_message {
            assert!(
                !msg.to_lowercase().contains("expect"),
                "Error message should not mention expect"
            );
        }

        let claude_not_found = ClaudeUsage::claude_not_available();
        if let Some(msg) = &claude_not_found.error_message {
            assert!(
                !msg.to_lowercase().contains("expect"),
                "Error message should not mention expect"
            );
        }
    }
}
