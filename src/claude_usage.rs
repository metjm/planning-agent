use crate::planning_paths;
use chrono::Local;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct ClaudeUsage {

    pub weekly_used: Option<u8>,

    pub session_used: Option<u8>,

    pub plan_type: Option<String>,

    pub fetched_at: Option<Instant>,

    pub error_message: Option<String>,
}

impl ClaudeUsage {

    #[allow(dead_code)]
    pub fn is_stale(&self) -> bool {
        match self.fetched_at {
            Some(t) => t.elapsed() > Duration::from_secs(300), 
            None => true,
        }
    }

    pub fn claude_not_available() -> Self {
        Self {
            error_message: Some("Claude CLI not found".to_string()),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn with_error(msg: String) -> Self {
        Self {
            error_message: Some(msg),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }
}

pub fn is_claude_available() -> bool {
    which::which("claude").is_ok()
}

/// Returns true if debug logging is enabled via CLAUDE_USAGE_DEBUG=1
fn is_debug_enabled() -> bool {
    env::var("CLAUDE_USAGE_DEBUG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get prompt timeout from env or use default (10 seconds)
fn get_prompt_timeout() -> Duration {
    env::var("CLAUDE_USAGE_PROMPT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(10))
}

/// Get usage output timeout from env or use default (8 seconds)
fn get_usage_timeout() -> Duration {
    env::var("CLAUDE_USAGE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(8))
}

/// Get overall timeout from env or use default (25 seconds)
fn get_overall_timeout() -> Duration {
    env::var("CLAUDE_USAGE_OVERALL_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(25))
}

/// Debug logger that writes to ~/.planning-agent/logs/claude-usage.log when CLAUDE_USAGE_DEBUG=1
struct DebugLogger {
    enabled: bool,
    start: Instant,
    entries: Vec<String>,
}

impl DebugLogger {
    fn new() -> Self {
        Self {
            enabled: is_debug_enabled(),
            start: Instant::now(),
            entries: Vec::new(),
        }
    }

    fn log(&mut self, message: &str) {
        if self.enabled {
            let elapsed_ms = self.start.elapsed().as_millis();
            self.entries.push(format!("[+{:06}ms] {}", elapsed_ms, message));
        }
    }

    fn log_output_snapshot(&mut self, label: &str, output: &str, max_bytes: usize) {
        if self.enabled {
            let truncated = if output.len() > max_bytes {
                format!("{}... (truncated, {} total bytes)", &output[..max_bytes], output.len())
            } else {
                output.to_string()
            };
            // Escape control characters for readability
            let escaped = truncated
                .replace('\r', "\\r")
                .replace('\n', "\\n")
                .replace('\x1b', "\\x1b");
            self.log(&format!("{}: {}", label, escaped));
        }
    }

    fn flush(&self) {
        if !self.enabled || self.entries.is_empty() {
            return;
        }

        if let Ok(log_path) = planning_paths::claude_usage_log_path() {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                let _ = writeln!(file, "\n=== Claude Usage Fetch: {} ===", timestamp);
                for entry in &self.entries {
                    let _ = writeln!(file, "{}", entry);
                }
                let _ = writeln!(file, "=== End ===\n");
            }
        }
    }
}

impl Drop for DebugLogger {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Result of analyzing Claude CLI output for special states
#[derive(Debug, Clone, PartialEq)]
enum CliState {
    /// Normal interactive prompt ready
    Ready,
    /// CLI requires login/authentication
    RequiresAuth,
    /// First-run setup flow
    FirstRun,
    /// Unknown/unexpected state
    Unknown(String),
}

/// Detect CLI state from output (auth required, first-run, etc.)
fn detect_cli_state(output: &str) -> CliState {
    let lower = output.to_lowercase();

    // Check for authentication-related messages
    if lower.contains("log in")
        || lower.contains("login")
        || lower.contains("authenticate")
        || lower.contains("sign in")
        || lower.contains("api key")
        || lower.contains("not logged in")
        || lower.contains("unauthorized")
    {
        return CliState::RequiresAuth;
    }

    // Check for first-run/setup indicators
    if lower.contains("welcome to claude")
        || lower.contains("first time")
        || lower.contains("getting started")
        || lower.contains("setup")
        || lower.contains("configure")
    {
        return CliState::FirstRun;
    }

    // Check for ready state (has a prompt character)
    let has_prompt = output.lines().any(|line| {
        let trimmed = line.trim();
        // Claude CLI prompt typically ends with '>' or contains a model indicator with '>'
        trimmed.ends_with('>')
            || (trimmed.contains('>') && (trimmed.contains("claude") || trimmed.contains("opus") || trimmed.contains("sonnet")))
    });

    if has_prompt {
        return CliState::Ready;
    }

    CliState::Unknown(if output.len() > 200 {
        format!("{}...", &output[..200])
    } else {
        output.to_string()
    })
}

fn run_claude_usage_via_pty(command: &str, _timeout: Duration) -> Result<String, String> {
    let mut logger = DebugLogger::new();
    logger.log(&format!("Starting Claude usage fetch, command: {}", command));

    let prompt_timeout = get_prompt_timeout();
    let usage_timeout = get_usage_timeout();
    let overall_timeout = get_overall_timeout();

    logger.log(&format!(
        "Timeouts: prompt={}s, usage={}s, overall={}s",
        prompt_timeout.as_secs(),
        usage_timeout.as_secs(),
        overall_timeout.as_secs()
    ));

    let pty_system = native_pty_system();
    logger.log("PTY system obtained");

    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to allocate PTY: {}", e))?;
    logger.log("PTY allocated");

    let cmd = CommandBuilder::new(command);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn Claude: {}", e))?;
    logger.log("Claude process spawned");

    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to get PTY reader: {}", e))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;
    logger.log("PTY reader/writer obtained");

    let output_buffer = Arc::new(Mutex::new(Vec::new()));
    let buffer_clone = output_buffer.clone();

    let reader_handle = std::thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buffer_clone.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
    });
    logger.log("Reader thread spawned");

    let start = Instant::now();

    // Phase 1: Wait for prompt
    logger.log("Phase 1: Waiting for prompt...");
    let mut cli_state;

    loop {
        if start.elapsed() > prompt_timeout {
            let data = output_buffer.lock().unwrap();
            let text = String::from_utf8_lossy(&data);
            let stripped = strip_ansi_codes(&text);
            drop(data);

            logger.log_output_snapshot("Output at prompt timeout", &stripped, 2048);

            // Check for special states before giving up
            cli_state = detect_cli_state(&stripped);
            logger.log(&format!("Detected CLI state: {:?}", cli_state));

            let _ = child.kill();
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();

            return match cli_state {
                CliState::RequiresAuth => Err("Claude CLI requires login. Run 'claude' to authenticate.".to_string()),
                CliState::FirstRun => Err("Claude CLI requires setup. Run 'claude' to complete first-time configuration.".to_string()),
                _ => Err("Timeout waiting for Claude CLI prompt".to_string()),
            };
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        let len = data.len();
        drop(data);

        cli_state = detect_cli_state(&stripped);

        // Early exit for auth/setup states
        if matches!(cli_state, CliState::RequiresAuth | CliState::FirstRun) {
            logger.log(&format!("Early detection of special state: {:?}", cli_state));
            logger.log_output_snapshot("Output at early detection", &stripped, 2048);

            let _ = child.kill();
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();

            return match cli_state {
                CliState::RequiresAuth => Err("Claude CLI requires login. Run 'claude' to authenticate.".to_string()),
                CliState::FirstRun => Err("Claude CLI requires setup. Run 'claude' to complete first-time configuration.".to_string()),
                _ => unreachable!(),
            };
        }

        if cli_state == CliState::Ready && len > 100 {
            logger.log(&format!("Prompt detected after {}ms, buffer size: {} bytes", start.elapsed().as_millis(), len));
            logger.log_output_snapshot("Output at prompt detection", &stripped, 1024);
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // Check overall timeout
    if start.elapsed() > overall_timeout {
        logger.log("Overall timeout exceeded after prompt detection");
        let _ = child.kill();
        drop(writer);
        drop(pair.master);
        let _ = reader_handle.join();
        return Err("Timeout waiting for Claude CLI".to_string());
    }

    // Phase 2: Send /usage command
    logger.log("Phase 2: Sending /usage command...");
    for c in "/usage".chars() {
        writer.write_all(&[c as u8]).map_err(|e| format!("Failed to send char: {}", e))?;
        std::thread::sleep(Duration::from_millis(50));
    }

    std::thread::sleep(Duration::from_millis(200));

    writer.write_all(b"\r").map_err(|e| format!("Failed to send Enter: {}", e))?;
    logger.log("/usage command sent");

    // Phase 3: Wait for usage output
    logger.log("Phase 3: Waiting for usage output...");
    let usage_start = Instant::now();
    let mut last_len = 0;
    let mut usage_found = false;

    loop {
        if usage_start.elapsed() > usage_timeout {
            logger.log("Usage output timeout reached");
            break;
        }

        let data = output_buffer.lock().unwrap();
        let len = data.len();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);

        let has_usage_output = stripped.contains('%')
            || stripped.to_lowercase().contains("limit")
            || stripped.to_lowercase().contains("used");
        drop(data);

        if len > last_len {
            last_len = len;
        } else if has_usage_output && usage_start.elapsed() > Duration::from_millis(1500) {
            usage_found = true;
            logger.log(&format!("Usage output detected after {}ms", usage_start.elapsed().as_millis()));
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if !usage_found {
        logger.log("No usage indicators found in output");
    }

    // Phase 4: Send /exit and cleanup
    logger.log("Phase 4: Sending /exit command...");
    for c in "/exit".chars() {
        let _ = writer.write_all(&[c as u8]);
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = writer.write_all(b"\r");

    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                logger.log(&format!("Process exited with status: {:?}", status));
                break;
            }
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    logger.log("Exit deadline reached, killing process");
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                logger.log(&format!("Error waiting for process: {}", e));
                let _ = child.kill();
                break;
            }
        }
    }

    drop(writer);
    drop(pair.master);
    let _ = reader_handle.join();

    let output = output_buffer.lock().unwrap().clone();
    let result = String::from_utf8_lossy(&output).into_owned();

    logger.log(&format!("Total fetch time: {}ms", start.elapsed().as_millis()));
    logger.log_output_snapshot("Final output", &strip_ansi_codes(&result), 4096);

    Ok(result)
}

fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {

            if chars.peek() == Some(&'[') {
                chars.next(); 
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

pub fn fetch_claude_usage_sync() -> ClaudeUsage {
    if !is_claude_available() {
        return ClaudeUsage::claude_not_available();
    }

    let timeout = Duration::from_secs(25);

    match run_claude_usage_via_pty("claude", timeout) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);

            let session = parse_usage_used_percent(&output, "current session");
            let weekly = parse_usage_used_percent(&output, "current week")
                .or_else(|| parse_usage_used_percent(&output, "week"));

            let plan = parse_plan_type(&output);

            ClaudeUsage {
                weekly_used: weekly,
                session_used: session,
                plan_type: plan,
                fetched_at: Some(Instant::now()),
                error_message: None,
            }
        }
        Err(e) => ClaudeUsage::with_error(e),
    }
}

fn parse_usage_used_percent(text: &str, section_keyword: &str) -> Option<u8> {
    let lines: Vec<&str> = text.lines().collect();
    let section_keyword_lower = section_keyword.to_lowercase();

    for (i, line) in lines.iter().enumerate() {
        let line_lower = line.to_lowercase();

        if line_lower.contains(&section_keyword_lower) {

            for j in i..std::cmp::min(i + 5, lines.len()) {
                let candidate = lines[j];
                if candidate.to_lowercase().contains("used") {

                    if let Some(pct_pos) = candidate.find('%') {
                        let before = &candidate[..pct_pos];
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
        }
    }
    None
}

fn parse_plan_type(text: &str) -> Option<String> {
    let text_lower = text.to_lowercase();

    for plan_name in &["Max", "Pro", "Free"] {
        let pattern = format!("claude {}", plan_name.to_lowercase());
        if text_lower.contains(&pattern) {
            return Some(plan_name.to_string());
        }
    }

    for line in text.lines() {
        let line_lower = line.to_lowercase();
        if line_lower.contains("plan") {
            if let Some(colon_pos) = line.find(':') {
                let after_colon = line[colon_pos + 1..].trim();
                if !after_colon.is_empty() {
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
    fn test_parse_usage_used_percent() {

        let output = r#"
 Current session
 ██▌                                                5% used
 Resets 9:59am (America/Los_Angeles)

 Current week (all models)
 ████████████████████▌                              41% used
 Resets Dec 26, 5:59am (America/Los_Angeles)
"#;
        assert_eq!(parse_usage_used_percent(output, "current session"), Some(5));
        assert_eq!(parse_usage_used_percent(output, "current week"), Some(41));
    }

    #[test]
    fn test_parse_usage_used_percent_100() {
        let output = r#"
 Current session
 ████████████████████████████████████████████████████100% used
"#;
        assert_eq!(parse_usage_used_percent(output, "current session"), Some(100));
    }

    #[test]
    fn test_parse_usage_used_percent_not_found() {
        let output = "No percentage here";
        assert_eq!(parse_usage_used_percent(output, "session"), None);
    }

    #[test]
    fn test_parse_plan_type_claude_max() {

        let output = "Opus 4.5 · Claude Max · gabe.b.azevedo@gmail.com's Organization";
        assert_eq!(parse_plan_type(output), Some("Max".to_string()));
    }

    #[test]
    fn test_parse_plan_type_claude_pro() {
        let output = "Sonnet · Claude Pro · user@example.com";
        assert_eq!(parse_plan_type(output), Some("Pro".to_string()));
    }

    #[test]
    fn test_parse_plan_type_fallback() {

        assert_eq!(parse_plan_type("Plan: Max"), Some("Max".to_string()));
        assert_eq!(
            parse_plan_type("Your plan: Pro tier"),
            Some("Pro".to_string())
        );
    }

    #[test]
    fn test_parse_plan_type_not_found() {
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

        assert_eq!(strip_ansi_codes("\x1b[32mHello\x1b[0m"), "Hello");
        assert_eq!(
            strip_ansi_codes("\x1b[1;31mBold Red\x1b[0m Text"),
            "Bold Red Text"
        );

        assert_eq!(strip_ansi_codes("Plain text"), "Plain text");

        assert_eq!(
            strip_ansi_codes("\x1b[33mYellow\x1b[0m \x1b[34mBlue\x1b[0m"),
            "Yellow Blue"
        );

        assert_eq!(strip_ansi_codes("\x1b[2K\x1b[1GLine"), "Line");
    }

    #[test]
    fn test_parse_usage_with_ansi_codes() {

        let raw = "\x1b[32mCurrent session\x1b[0m\n██ 80% used";
        let stripped = strip_ansi_codes(raw);
        assert_eq!(parse_usage_used_percent(&stripped, "current session"), Some(80));
    }

    #[test]
    fn test_parse_plan_with_ansi_codes() {
        let raw = "\x1b[1mClaude Max\x1b[0m · user@example.com";
        let stripped = strip_ansi_codes(raw);
        assert_eq!(parse_plan_type(&stripped), Some("Max".to_string()));
    }

    #[test]
    fn test_no_expect_in_error_messages() {

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

    #[test]
    fn test_claude_usage_with_error_sets_fetched_at() {
        let usage = ClaudeUsage::with_error("Test error".to_string());
        assert!(
            usage.fetched_at.is_some(),
            "with_error should set fetched_at"
        );
        assert_eq!(usage.error_message, Some("Test error".to_string()));
    }

    #[test]
    fn test_claude_usage_not_available_sets_fetched_at() {
        let usage = ClaudeUsage::claude_not_available();
        assert!(
            usage.fetched_at.is_some(),
            "claude_not_available should set fetched_at"
        );
        assert_eq!(
            usage.error_message,
            Some("Claude CLI not found".to_string())
        );
    }

    // Tests for CLI state detection
    #[test]
    fn test_detect_cli_state_ready() {
        // Standard Claude CLI prompt
        let output = "Opus 4.5 · Claude Max · user@example.com >";
        assert_eq!(detect_cli_state(output), CliState::Ready);

        // Sonnet prompt
        let output = "Sonnet · Claude Pro > ";
        assert_eq!(detect_cli_state(output), CliState::Ready);

        // Just > character
        let output = "Loading...\n>";
        assert_eq!(detect_cli_state(output), CliState::Ready);
    }

    #[test]
    fn test_detect_cli_state_requires_auth() {
        assert_eq!(
            detect_cli_state("Please log in to continue"),
            CliState::RequiresAuth
        );
        assert_eq!(
            detect_cli_state("You are not logged in"),
            CliState::RequiresAuth
        );
        assert_eq!(
            detect_cli_state("Please authenticate first"),
            CliState::RequiresAuth
        );
        assert_eq!(
            detect_cli_state("API key required"),
            CliState::RequiresAuth
        );
    }

    #[test]
    fn test_detect_cli_state_first_run() {
        assert_eq!(
            detect_cli_state("Welcome to Claude Code! Let's get started."),
            CliState::FirstRun
        );
        assert_eq!(
            detect_cli_state("First time setup required"),
            CliState::FirstRun
        );
        assert_eq!(
            detect_cli_state("Configure your settings"),
            CliState::FirstRun
        );
    }

    #[test]
    fn test_detect_cli_state_unknown() {
        // No clear indicators
        let state = detect_cli_state("Loading spinner...");
        assert!(matches!(state, CliState::Unknown(_)));

        // Empty output
        let state = detect_cli_state("");
        assert!(matches!(state, CliState::Unknown(_)));
    }

    // Tests for env var timeout configuration
    #[test]
    fn test_default_timeouts() {
        // These tests verify defaults when env vars are not set
        // Note: actual env var testing would require setting/unsetting vars
        let prompt = get_prompt_timeout();
        assert_eq!(prompt.as_secs(), 10);

        let usage = get_usage_timeout();
        assert_eq!(usage.as_secs(), 8);

        let overall = get_overall_timeout();
        assert_eq!(overall.as_secs(), 25);
    }

    #[test]
    fn test_is_debug_enabled_default() {
        // By default, debug should be disabled (env var not set in test environment)
        // This test verifies the function doesn't panic
        let _ = is_debug_enabled();
    }

    // Tests for debug logger (unit level)
    #[test]
    fn test_debug_logger_disabled() {
        // When debug is disabled, entries should still be collected but not written
        let mut logger = DebugLogger::new();
        logger.log("Test message");
        logger.log_output_snapshot("Test", "some output", 100);
        // Should not panic, logger will be dropped without writing when disabled
    }

    #[test]
    fn test_debug_logger_output_snapshot_truncation() {
        let mut logger = DebugLogger::new();
        let long_output = "a".repeat(1000);
        logger.log_output_snapshot("Long output", &long_output, 100);
        // Verify the logger handles long output without panicking
    }

    #[test]
    #[ignore]
    fn test_fetch_claude_usage_real() {
        if !is_claude_available() {
            eprintln!("Claude CLI not found, skipping integration test");
            return;
        }

        eprintln!("Fetching real Claude usage (this may take 15-20 seconds)...");
        let usage = fetch_claude_usage_sync();

        eprintln!("Result: {:?}", usage);

        assert!(usage.fetched_at.is_some(), "fetched_at should be set");

        if usage.error_message.is_none() {

            let has_data = usage.session_used.is_some()
                || usage.weekly_used.is_some()
                || usage.plan_type.is_some();
            assert!(has_data, "Should have at least some usage data: {:?}", usage);

            if let Some(session) = usage.session_used {
                eprintln!("Session used: {}%", session);
                assert!(session <= 100, "Session percentage should be <= 100");
            }
            if let Some(weekly) = usage.weekly_used {
                eprintln!("Weekly used: {}%", weekly);
                assert!(weekly <= 100, "Weekly percentage should be <= 100");
            }
            if let Some(ref plan) = usage.plan_type {
                eprintln!("Plan type: {}", plan);
            }
        } else {
            eprintln!("Got error (may be expected): {:?}", usage.error_message);
        }
    }

    #[test]
    #[ignore]
    fn test_fetch_claude_usage_with_debug_logging() {
        // Set CLAUDE_USAGE_DEBUG=1 before running this test manually:
        // CLAUDE_USAGE_DEBUG=1 cargo test test_fetch_claude_usage_with_debug_logging --release -- --ignored
        if !is_claude_available() {
            eprintln!("Claude CLI not found, skipping integration test");
            return;
        }

        eprintln!("Fetching Claude usage with debug logging enabled...");
        eprintln!("Debug logs will be written to ~/.planning-agent/logs/claude-usage.log");

        let usage = fetch_claude_usage_sync();
        eprintln!("Result: {:?}", usage);

        // Check if log file was created (when debug is enabled)
        if is_debug_enabled() {
            if let Ok(log_path) = planning_paths::claude_usage_log_path() {
                assert!(log_path.exists(), "Debug log file should exist when CLAUDE_USAGE_DEBUG=1");
                eprintln!("Debug log written to: {:?}", log_path);
            }
        }
    }
}
