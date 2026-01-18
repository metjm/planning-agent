use crate::planning_paths;
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct ClaudeUsage {
    /// Session usage with reset timestamp
    pub session: UsageWindow,

    /// Weekly usage with reset timestamp
    pub weekly: UsageWindow,

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
                let prefix = truncate_to_bytes_boundary(output, max_bytes);
                format!("{}... (truncated, {} total bytes)", prefix, output.len())
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

    // Check for first-run/setup indicators using SPECIFIC patterns only.
    // Generic words like "setup" and "configure" can appear in normal CLI conversation
    // output (e.g., user discussing "setup instructions" for their project).
    // These specific patterns are unique to Claude CLI onboarding prompts.
    //
    // NOTE: We removed "getting started" because the Claude CLI welcome screen
    // always shows "Tips for getting started" in the sidebar, even for properly
    // configured users. This was causing false positives.
    if lower.contains("welcome to claude")
        || lower.contains("first time")
        // Specific setup phrases that indicate onboarding
        || lower.contains("complete setup")
        || lower.contains("finish setup")
        || lower.contains("initial setup")
        || lower.contains("setup required")
        || lower.contains("setup is required")
        // Specific configure phrases that indicate onboarding
        || lower.contains("configure claude")
        || lower.contains("configuration required")
    {
        return CliState::FirstRun;
    }

    // Check for ready state indicators
    // 1. "Welcome back" in the welcome box indicates user is logged in and CLI is ready
    // 2. The CLI prompt uses '❯' (unicode chevron) or '>' character
    // 3. Model indicator line with model names
    if lower.contains("welcome back") {
        return CliState::Ready;
    }

    let has_prompt = output.lines().any(|line| {
        let trimmed = line.trim();
        // Claude CLI v2.x uses '❯' (unicode chevron U+276F) for the input prompt
        // Older versions may use '>'
        trimmed.ends_with('>')
            || trimmed.ends_with('❯')
            || trimmed.starts_with('❯')
            || (trimmed.contains('>') && (trimmed.contains("claude") || trimmed.contains("opus") || trimmed.contains("sonnet")))
    });

    if has_prompt {
        return CliState::Ready;
    }

    // Return Unknown with sanitized excerpt for diagnostic purposes
    let sanitized = strip_ansi_codes(output);
    let excerpt = truncate_for_excerpt(&sanitized, 100);
    CliState::Unknown(excerpt)
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
                CliState::Unknown(excerpt) => Err(format!(
                    "Unable to determine CLI state (set CLAUDE_USAGE_DEBUG=1 for details). Output: {}",
                    excerpt
                )),
                CliState::Ready => Err("Timeout waiting for Claude CLI prompt".to_string()),
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

fn truncate_to_bytes_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }

    let mut last_index = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        last_index = next;
    }
    // last_index is guaranteed to be at a valid UTF-8 char boundary
    // because we only update it via char_indices()
    text.get(..last_index).unwrap_or("")
}

fn truncate_for_excerpt(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

pub fn fetch_claude_usage_sync() -> ClaudeUsage {
    if !is_claude_available() {
        return ClaudeUsage::claude_not_available();
    }

    let timeout = Duration::from_secs(25);

    match run_claude_usage_via_pty("claude", timeout) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);

            let session_pct = parse_usage_used_percent(&output, "current session");
            let weekly_pct = parse_usage_used_percent(&output, "current week")
                .or_else(|| parse_usage_used_percent(&output, "week"));

            let session_reset = parse_reset_timestamp(&output, "current session");
            let weekly_reset = parse_reset_timestamp(&output, "current week")
                .or_else(|| parse_reset_timestamp(&output, "week"));

            let plan = parse_plan_type(&output);

            // Build usage windows with reset timestamps and spans
            // Claude session window length is not verifiable from CLI output, so use Unknown
            // Weekly window is known to be 7 days
            let session = match (session_pct, session_reset) {
                (Some(pct), Some(ts)) => UsageWindow::with_percent_reset_and_span(
                    pct,
                    ts,
                    UsageWindowSpan::Unknown, // Session duration not verifiable
                ),
                (Some(pct), None) => UsageWindow::with_percent_and_span(pct, UsageWindowSpan::Unknown),
                _ => UsageWindow::default(),
            };
            let weekly = match (weekly_pct, weekly_reset) {
                (Some(pct), Some(ts)) => UsageWindow::with_percent_reset_and_span(
                    pct,
                    ts,
                    UsageWindowSpan::Days(7), // Weekly window is 7 days
                ),
                (Some(pct), None) => UsageWindow::with_percent_and_span(pct, UsageWindowSpan::Days(7)),
                _ => UsageWindow::default(),
            };

            ClaudeUsage {
                session,
                weekly,
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

            for candidate in lines.iter().skip(i).take(5) {
                if candidate.to_lowercase().contains("used") {
                    // '%' is ASCII, so pct_pos is guaranteed to be at a valid UTF-8 boundary
                    if let Some(pct_pos) = candidate.find('%') {
                        let before = candidate.get(..pct_pos).unwrap_or("");
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
            // ':' is ASCII, so colon_pos + 1 is guaranteed to be at a valid UTF-8 boundary
            if let Some(colon_pos) = line.find(':') {
                let after_colon = line.get(colon_pos + 1..).unwrap_or("").trim();
                if !after_colon.is_empty() {
                    let plan = after_colon.split_whitespace().next()?;
                    return Some(plan.to_string());
                }
            }
        }
    }

    None
}

/// Parse reset timestamp from Claude /usage output for a given section.
///
/// Looks for patterns like:
/// - "Resets 9:59am (America/Los_Angeles)" (time-only)
/// - "Resets Dec 26, 5:59am (America/Los_Angeles)" (date without year)
fn parse_reset_timestamp(text: &str, section_keyword: &str) -> Option<ResetTimestamp> {
    let lines: Vec<&str> = text.lines().collect();
    let section_keyword_lower = section_keyword.to_lowercase();

    for (i, line) in lines.iter().enumerate() {
        let line_lower = line.to_lowercase();

        if line_lower.contains(&section_keyword_lower) {
            // Look for "Resets" line in the next few lines
            for candidate in lines.iter().skip(i).take(5) {
                if let Some(ts) = parse_reset_line(candidate) {
                    return Some(ts);
                }
            }
        }
    }
    None
}

/// Parse a single "Resets ..." line and extract the timestamp.
///
/// Handles two formats:
/// 1. Time-only: "Resets 9:59am (America/Los_Angeles)"
/// 2. Date+time: "Resets Dec 26, 5:59am (America/Los_Angeles)"
fn parse_reset_line(line: &str) -> Option<ResetTimestamp> {
    // Find "Resets" keyword (case-insensitive)
    let lower = line.to_lowercase();
    let resets_pos = lower.find("resets ")?;
    // "resets " is 7 ASCII characters, so resets_pos + 7 is at a valid UTF-8 boundary
    let after_resets = line.get(resets_pos + 7..)?;

    // Extract timezone from parentheses
    // '(' and ')' are ASCII, so tz_start and tz_end are at valid UTF-8 boundaries
    let tz_start = after_resets.find('(')?;
    let tz_end = after_resets.find(')')?;
    if tz_start >= tz_end {
        return None;
    }

    let tz_str = after_resets.get(tz_start + 1..tz_end)?.trim();
    let time_part = after_resets.get(..tz_start)?.trim();

    // Parse timezone
    let tz: Tz = Tz::from_str(tz_str).ok()?;

    // Get current time in the target timezone
    let now_utc = Utc::now();
    let now_tz = now_utc.with_timezone(&tz);

    // Try to parse time and optional date
    if let Some(ts) = parse_datetime_with_date(time_part, tz, now_tz) {
        return Some(ts);
    }

    if let Some(ts) = parse_time_only(time_part, tz, now_tz) {
        return Some(ts);
    }

    None
}

/// Parse a date+time string like "Dec 26, 5:59am" or "Jan 1, 12:00pm"
fn parse_datetime_with_date(
    time_part: &str,
    tz: Tz,
    now_tz: chrono::DateTime<Tz>,
) -> Option<ResetTimestamp> {
    // Look for month abbreviation at the start
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];

    let lower = time_part.to_lowercase();
    let month_idx = months.iter().position(|m| lower.starts_with(m))?;

    // Extract day number - find digits after month
    // Month abbreviations (jan, feb, etc.) are 3 ASCII characters, so index 3 is valid
    let after_month = time_part.get(3..)?.trim_start();

    // Find the comma that separates day from time
    // ',' is ASCII, so comma_pos and comma_pos + 1 are at valid UTF-8 boundaries
    let comma_pos = after_month.find(',')?;
    let day_str = after_month.get(..comma_pos)?.trim();
    let time_str = after_month.get(comma_pos + 1..)?.trim();

    let day: u32 = day_str.parse().ok()?;

    // Parse time (e.g., "5:59am", "12:00pm")
    let naive_time = parse_am_pm_time(time_str)?;

    // Determine year - if month/day already passed, use next year
    let mut year = now_tz.year();
    let month = (month_idx + 1) as u32;

    let candidate_date = NaiveDate::from_ymd_opt(year, month, day)?;
    let candidate_dt = candidate_date.and_time(naive_time);

    // Convert to timezone, handling DST ambiguity with earliest()
    let local_dt = tz.from_local_datetime(&candidate_dt).earliest()?;

    // If this time is in the past, roll to next year
    if local_dt < now_tz {
        year += 1;
        let next_date = NaiveDate::from_ymd_opt(year, month, day)?;
        let next_dt = next_date.and_time(naive_time);
        let next_local = tz.from_local_datetime(&next_dt).earliest()?;
        return Some(ResetTimestamp::from_epoch_seconds(next_local.timestamp()));
    }

    Some(ResetTimestamp::from_epoch_seconds(local_dt.timestamp()))
}

/// Parse a time-only string like "9:59am" or "12:00pm"
fn parse_time_only(
    time_part: &str,
    tz: Tz,
    now_tz: chrono::DateTime<Tz>,
) -> Option<ResetTimestamp> {
    let naive_time = parse_am_pm_time(time_part)?;

    // Combine with today's date in the target timezone
    let today = now_tz.date_naive();
    let candidate_dt = today.and_time(naive_time);

    // Convert to timezone, handling DST ambiguity with earliest()
    let local_dt = tz.from_local_datetime(&candidate_dt).earliest()?;

    // If this time is in the past, roll to tomorrow
    if local_dt < now_tz {
        let tomorrow = today.succ_opt()?;
        let tomorrow_dt = tomorrow.and_time(naive_time);
        let next_local = tz.from_local_datetime(&tomorrow_dt).earliest()?;
        return Some(ResetTimestamp::from_epoch_seconds(next_local.timestamp()));
    }

    Some(ResetTimestamp::from_epoch_seconds(local_dt.timestamp()))
}

/// Parse time in am/pm format like "9:59am", "12:00pm", "5:30AM"
fn parse_am_pm_time(time_str: &str) -> Option<NaiveTime> {
    let lower = time_str.to_lowercase();
    let is_pm = lower.contains("pm");
    let is_am = lower.contains("am");

    if !is_pm && !is_am {
        return None;
    }

    // Remove am/pm suffix
    let cleaned = lower.replace("am", "").replace("pm", "").trim().to_string();

    // Parse hour:minute
    let parts: Vec<&str> = cleaned.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let mut hour: u32 = parts[0].trim().parse().ok()?;
    let minute: u32 = parts[1].trim().parse().ok()?;

    // Convert to 24-hour format
    if is_pm && hour != 12 {
        hour += 12;
    } else if is_am && hour == 12 {
        hour = 0;
    }

    NaiveTime::from_hms_opt(hour, minute, 0)
}

#[cfg(test)]
#[path = "claude_usage_tests.rs"]
mod tests;
