//! Codex CLI usage tracking via /status command

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


#[derive(Debug, Clone, Default)]
pub struct CodexUsage {
    /// 5-hour limit remaining as percentage
    pub hourly_remaining: Option<u8>,
    /// Weekly limit remaining as percentage
    pub weekly_remaining: Option<u8>,
    /// Account type (e.g., "Pro")
    pub plan_type: Option<String>,
    /// When this data was fetched
    pub fetched_at: Option<Instant>,
    /// Error message if fetch failed
    pub error_message: Option<String>,
}

impl CodexUsage {
    pub fn with_error(error: String) -> Self {
        Self {
            error_message: Some(error),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn not_available() -> Self {
        Self {
            error_message: Some("CLI not found".to_string()),
            fetched_at: Some(Instant::now()),
            ..Default::default()
        }
    }
}

/// Check if Codex CLI is available
pub fn is_codex_available() -> bool {
    which::which("codex").is_ok()
}

/// Fetch Codex usage by running /status command via PTY
pub fn fetch_codex_usage_sync() -> CodexUsage {
    if !is_codex_available() {
        return CodexUsage::not_available();
    }

    let timeout = Duration::from_secs(20);

    match run_codex_status_via_pty("codex", timeout) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);

            // Parse usage from /status output
            match parse_codex_usage(&output) {
                ParseResult::Success {
                    hourly,
                    weekly,
                    plan,
                } => CodexUsage {
                    hourly_remaining: hourly,
                    weekly_remaining: weekly,
                    plan_type: plan,
                    fetched_at: Some(Instant::now()),
                    error_message: None,
                },
                ParseResult::UsageLimitHit => CodexUsage {
                    error_message: Some("Usage limit hit".to_string()),
                    fetched_at: Some(Instant::now()),
                    ..Default::default()
                },
                ParseResult::DataNotAvailable => CodexUsage {
                    error_message: Some("Data not available yet".to_string()),
                    fetched_at: Some(Instant::now()),
                    ..Default::default()
                },
                ParseResult::UnrecognizedFormat => CodexUsage {
                    error_message: Some("Could not parse status".to_string()),
                    fetched_at: Some(Instant::now()),
                    ..Default::default()
                },
            }
        }
        Err(e) => CodexUsage::with_error(e),
    }
}

/// Poll-based read with timeout for Unix systems
/// Returns true if data is available, false on timeout
/// If fd is None (non-Unix or unavailable), returns true (no timeout support)
#[cfg(unix)]
fn poll_read_ready(fd: Option<i32>, timeout_ms: i32) -> Result<bool, String> {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
    use std::os::unix::io::BorrowedFd;

    let Some(raw_fd) = fd else {
        // No fd available, skip poll and proceed with read
        return Ok(true);
    };

    // SAFETY: The fd is valid for the duration of this call as it comes from an open PTY
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(raw_fd) };
    let mut poll_fds = [PollFd::new(borrowed_fd, PollFlags::POLLIN)];

    let timeout = if timeout_ms < 0 {
        PollTimeout::NONE
    } else {
        PollTimeout::try_from(timeout_ms as u16).unwrap_or(PollTimeout::MAX)
    };

    match poll(&mut poll_fds, timeout) {
        Ok(0) => Ok(false), // Timeout - no data available
        Ok(_) => Ok(true),  // Data available
        Err(e) => Err(format!("Poll error: {}", e)),
    }
}

/// Fallback for non-Unix: always returns true (no timeout support)
#[cfg(not(unix))]
fn poll_read_ready(_fd: Option<i32>, _timeout_ms: i32) -> Result<bool, String> {
    Ok(true)
}

/// Run Codex CLI and execute /status command
fn run_codex_status_via_pty(command: &str, timeout: Duration) -> Result<String, String> {
    let pty_system = native_pty_system();

    // Use standard terminal size - Codex is picky about PTY settings
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
        .map_err(|e| format!("Failed to spawn Codex: {}", e))?;

    drop(pair.slave);

    // Get the raw fd from master BEFORE cloning reader (Unix only)
    // portable-pty's as_raw_fd() returns Option<RawFd>
    #[cfg(unix)]
    let master_fd: Option<i32> = pair.master.as_raw_fd();
    #[cfg(not(unix))]
    let master_fd: Option<i32> = None;

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("Failed to get PTY reader: {}", e))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

    let output_buffer = Arc::new(Mutex::new(Vec::new()));
    let buffer_clone = output_buffer.clone();

    // Flag to signal when cursor position query is detected
    let needs_cursor_response = Arc::new(Mutex::new(false));
    let needs_cursor_clone = needs_cursor_response.clone();

    // Flag to signal reader thread to stop
    let stop_reader = Arc::new(AtomicBool::new(false));
    let stop_reader_clone = stop_reader.clone();

    // Per-read timeout for the reader thread (500ms between poll checks)
    let read_poll_timeout_ms = 500i32;

    let reader_handle = std::thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 1024];
        loop {
            // Check if we should stop
            if stop_reader_clone.load(Ordering::Relaxed) {
                break;
            }

            // Poll for data availability with timeout
            match poll_read_ready(master_fd, read_poll_timeout_ms) {
                Ok(false) => {
                    // Timeout - no data available, check stop flag and continue
                    continue;
                }
                Ok(true) => {
                    // Data available, proceed with read
                }
                Err(_) => {
                    // Poll error, exit loop
                    break;
                }
            }

            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &chunk[..n];
                    buffer_clone.lock().unwrap().extend_from_slice(data);

                    // Check for cursor position query (DSR)
                    // Codex sends \x1b[6n and expects \x1b[row;colR response
                    let text = String::from_utf8_lossy(data);
                    if text.contains("\x1b[6n") || text.contains("[6n") {
                        *needs_cursor_clone.lock().unwrap() = true;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Helper to check and respond to cursor position queries
    let respond_to_cursor_query = |w: &mut Box<dyn Write + Send>, flag: &Arc<Mutex<bool>>| {
        let mut needs_response = flag.lock().unwrap();
        if *needs_response {
            // Respond with cursor at position 1,1
            let _ = w.write_all(b"\x1b[1;1R");
            *needs_response = false;
        }
    };

    let start = Instant::now();
    let prompt_timeout = Duration::from_secs(15);

    // Wait for Codex prompt
    loop {
        if start.elapsed() > prompt_timeout {
            let _ = child.kill();
            stop_reader.store(true, Ordering::Relaxed);
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();
            return Err("Timeout waiting for Codex CLI prompt".to_string());
        }

        // Respond to cursor position queries
        respond_to_cursor_query(&mut writer, &needs_cursor_response);

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        let len = data.len();
        drop(data);

        // Look for Codex prompt indicators
        // Codex shows "OpenAI Codex" banner and "Tip:" or "context left"
        let has_prompt = stripped.contains("OpenAI Codex")
            || stripped.contains("context left")
            || stripped.contains("Tip:")
            || stripped.contains("for shortcuts");

        if has_prompt && len > 200 {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if start.elapsed() > timeout {
        let _ = child.kill();
        stop_reader.store(true, Ordering::Relaxed);
        drop(writer);
        drop(pair.master);
        let _ = reader_handle.join();
        return Err("Timeout".to_string());
    }

    // Send /status command
    for c in "/status".chars() {
        writer.write_all(&[c as u8]).map_err(|e| format!("Failed to send: {}", e))?;
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(200));
    writer.write_all(b"\r").map_err(|e| format!("Failed to send Enter: {}", e))?;

    // Wait for status output
    let status_start = Instant::now();
    let status_timeout = Duration::from_secs(5);

    loop {
        if status_start.elapsed() > status_timeout {
            break;
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        drop(data);

        // Check if we have status output (look for limit indicators)
        if stripped.contains("Weekly limit") || stripped.contains("5h limit") {
            std::thread::sleep(Duration::from_millis(500)); // Let it finish
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // Exit with Ctrl+C (Codex uses this to exit)
    let _ = writer.write_all(&[0x03]); // Ctrl+C
    std::thread::sleep(Duration::from_millis(200));

    // Also try /exit
    for c in "/exit".chars() {
        let _ = writer.write_all(&[c as u8]);
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = writer.write_all(b"\r");

    let exit_deadline = Instant::now() + Duration::from_secs(3);
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

    // Signal reader thread to stop and wait for it
    stop_reader.store(true, Ordering::Relaxed);
    drop(writer);
    drop(pair.master);
    let _ = reader_handle.join();

    let output = output_buffer.lock().unwrap().clone();
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Strip ANSI escape sequences
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

/// Result of parsing Codex /status output
#[derive(Debug, Clone, PartialEq)]
pub enum ParseResult {
    /// Successfully parsed usage data
    Success {
        hourly: Option<u8>,
        weekly: Option<u8>,
        plan: Option<String>,
    },
    /// Usage limit hit - no data available
    UsageLimitHit,
    /// Data not available yet (Codex shows this on fresh accounts)
    DataNotAvailable,
    /// No recognizable format found
    UnrecognizedFormat,
}

/// Parse Codex usage from /status output
/// Looks for lines like:
///   "5h limit:         [█████████████████░░░] 84% left (resets 07:09)"
///   "Weekly limit:     [██████████████████░░] 89% left (resets 09:37 on 28 Dec)"
///   "Account:          email@example.com (Pro)"
///   "Limits:           data not available yet"
///   "You've hit your usage limit..."
pub fn parse_codex_usage(text: &str) -> ParseResult {
    let text_lower = text.to_lowercase();

    // Check for "usage limit hit" message first
    if text_lower.contains("hit your usage limit")
        || text_lower.contains("you've hit your")
        || text_lower.contains("usage limit reached")
    {
        return ParseResult::UsageLimitHit;
    }

    // Check for "data not available yet" message
    if text_lower.contains("data not available yet")
        || text_lower.contains("not available yet")
        || (text_lower.contains("limits:") && text_lower.contains("not available"))
    {
        return ParseResult::DataNotAvailable;
    }

    let mut hourly: Option<u8> = None;
    let mut weekly: Option<u8> = None;
    let mut plan: Option<String> = None;

    for line in text.lines() {
        let line_lower = line.to_lowercase();

        // Parse 5h limit
        if line_lower.contains("5h limit") {
            if let Some(pct) = extract_percentage_left(line) {
                hourly = Some(pct);
            }
        }

        // Parse weekly limit
        if line_lower.contains("weekly limit") {
            if let Some(pct) = extract_percentage_left(line) {
                weekly = Some(pct);
            }
        }

        // Parse account/plan type - look for "(Pro)" or similar
        if line_lower.contains("account:") {
            // Extract plan type from parentheses, e.g., "(Pro)"
            if let Some(start) = line.rfind('(') {
                if let Some(end) = line.rfind(')') {
                    if end > start {
                        let plan_str = &line[start + 1..end];
                        // Filter out "Unknown" and empty strings
                        if !plan_str.is_empty()
                            && !plan_str.contains('@')
                            && plan_str.to_lowercase() != "unknown"
                        {
                            plan = Some(plan_str.to_string());
                        }
                    }
                }
            }
        }
    }

    // If we found any data, return Success; otherwise UnrecognizedFormat
    if hourly.is_some() || weekly.is_some() {
        ParseResult::Success {
            hourly,
            weekly,
            plan,
        }
    } else {
        ParseResult::UnrecognizedFormat
    }
}

/// Extract percentage from "X% left" pattern
fn extract_percentage_left(line: &str) -> Option<u8> {
    // Look for "X% left" pattern
    if let Some(left_pos) = line.find("% left") {
        let before = &line[..left_pos];
        // Find the number before "% left"
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
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_codex_usage() {
        let output = r#"
│  5h limit:         [█████████████████░░░] 84% left (resets 07:09)           │
│  Weekly limit:     [██████████████████░░] 89% left (resets 09:37 on 28 Dec) │
│  Account:          r8b9dzx8qv@privaterelay.appleid.com (Pro)                │
"#;
        let result = parse_codex_usage(output);
        assert_eq!(
            result,
            ParseResult::Success {
                hourly: Some(84),
                weekly: Some(89),
                plan: Some("Pro".to_string()),
            }
        );
    }

    #[test]
    fn test_parse_codex_usage_low() {
        let output = r#"
  5h limit:         [██░░░░░░░░░░░░░░░░░░] 10% left (resets 12:00)
  Weekly limit:     [█░░░░░░░░░░░░░░░░░░░] 5% left (resets tomorrow)
  Account:          user@example.com (Plus)
"#;
        let result = parse_codex_usage(output);
        assert_eq!(
            result,
            ParseResult::Success {
                hourly: Some(10),
                weekly: Some(5),
                plan: Some("Plus".to_string()),
            }
        );
    }

    #[test]
    fn test_parse_codex_usage_limit_hit() {
        let output = "You've hit your usage limit. Upgrade to Pro for more.";
        assert_eq!(parse_codex_usage(output), ParseResult::UsageLimitHit);

        let output2 = "Usage limit reached. Please try again later.";
        assert_eq!(parse_codex_usage(output2), ParseResult::UsageLimitHit);
    }

    #[test]
    fn test_parse_codex_usage_data_not_available() {
        let output = "Limits: data not available yet";
        assert_eq!(parse_codex_usage(output), ParseResult::DataNotAvailable);

        let output2 = "Your usage data is not available yet.";
        assert_eq!(parse_codex_usage(output2), ParseResult::DataNotAvailable);
    }

    #[test]
    fn test_parse_codex_usage_unrecognized_format() {
        let output = "Some random output that doesn't match any pattern";
        assert_eq!(parse_codex_usage(output), ParseResult::UnrecognizedFormat);
    }

    #[test]
    fn test_codex_usage_with_error_sets_fetched_at() {
        let usage = CodexUsage::with_error("Test error".to_string());
        assert!(usage.fetched_at.is_some(), "with_error should set fetched_at");
        assert_eq!(usage.error_message, Some("Test error".to_string()));
    }

    #[test]
    fn test_codex_usage_not_available_sets_fetched_at() {
        let usage = CodexUsage::not_available();
        assert!(
            usage.fetched_at.is_some(),
            "not_available should set fetched_at"
        );
        assert_eq!(usage.error_message, Some("CLI not found".to_string()));
    }

    #[test]
    fn test_extract_percentage() {
        assert_eq!(extract_percentage_left("84% left"), Some(84));
        assert_eq!(extract_percentage_left("[███] 100% left (resets)"), Some(100));
        assert_eq!(extract_percentage_left("no percentage"), None);
    }

    /// Integration test - run with: cargo test test_fetch_codex_usage_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn test_fetch_codex_usage_real() {
        if !is_codex_available() {
            eprintln!("Codex CLI not found, skipping");
            return;
        }

        eprintln!("Fetching real Codex usage...");
        let usage = fetch_codex_usage_sync();
        eprintln!("Result: {:?}", usage);

        assert!(usage.fetched_at.is_some());
        if usage.error_message.is_none() {
            eprintln!("5h remaining: {:?}%", usage.hourly_remaining);
            eprintln!("Weekly remaining: {:?}%", usage.weekly_remaining);
            eprintln!("Plan: {:?}", usage.plan_type);
        }
    }
}
