
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn is_debug_enabled() -> bool {
    std::env::var("CODEX_USAGE_DEBUG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn write_debug_log(raw_output: &str, stripped_output: &str, parse_result: &str) {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;

    let dir = ".planning-agent";
    let _ = fs::create_dir_all(dir);

    let log_path = format!("{}/codex-status.log", dir);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let now = std::time::SystemTime::now();
        let _ = writeln!(file, "\n================================================================================");
        let _ = writeln!(file, "=== Codex Usage Debug Log - {:?} ===", now);
        let _ = writeln!(file, "================================================================================");
        let _ = writeln!(file, "\n--- Parse Result ---");
        let _ = writeln!(file, "{}", parse_result);
        let _ = writeln!(
            file,
            "\n--- Raw PTY Output ({} bytes) ---",
            raw_output.len()
        );
        let _ = writeln!(file, "{}", raw_output);
        let _ = writeln!(
            file,
            "\n--- Stripped Output ({} bytes) ---",
            stripped_output.len()
        );
        let _ = writeln!(file, "{}", stripped_output);
        let _ = writeln!(file, "\n--- Raw Bytes (hex) ---");
        for (i, chunk) in raw_output.as_bytes().chunks(64).enumerate() {
            let hex: String = chunk.iter().map(|b| format!("{:02x} ", b)).collect();
            let _ = writeln!(file, "{:04x}: {}", i * 64, hex);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CodexUsage {

    pub hourly_remaining: Option<u8>,

    pub weekly_remaining: Option<u8>,

    pub plan_type: Option<String>,

    pub fetched_at: Option<Instant>,

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

pub fn is_codex_available() -> bool {
    which::which("codex").is_ok()
}

pub fn fetch_codex_usage_sync() -> CodexUsage {
    if !is_codex_available() {
        return CodexUsage::not_available();
    }

    let timeout = Duration::from_secs(20);

    match run_codex_status_via_pty("codex", timeout) {
        Ok(raw_output) => {
            let output = normalize_status_output(&raw_output);
            let parse_result = parse_codex_usage(&output);

            if is_debug_enabled() {
                let result_str = format!("{:?}", parse_result);
                write_debug_log(&raw_output, &output, &result_str);
            }

            match parse_result {
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
                ParseResult::UnrecognizedFormat => {
                    if is_debug_enabled() {
                        write_debug_log(
                            &raw_output,
                            &output,
                            "UnrecognizedFormat - PARSE FAILURE",
                        );
                    }
                    CodexUsage {
                        error_message: Some("Could not parse status".to_string()),
                        fetched_at: Some(Instant::now()),
                        ..Default::default()
                    }
                }
            }
        }
        Err(e) => CodexUsage::with_error(e),
    }
}

#[cfg(unix)]
fn poll_read_ready(fd: Option<i32>, timeout_ms: i32) -> Result<bool, String> {
    use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
    use std::os::unix::io::BorrowedFd;

    let Some(raw_fd) = fd else {

        return Ok(true);
    };

    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(raw_fd) };
    let mut poll_fds = [PollFd::new(borrowed_fd, PollFlags::POLLIN)];

    let timeout = if timeout_ms < 0 {
        PollTimeout::NONE
    } else {
        PollTimeout::try_from(timeout_ms as u16).unwrap_or(PollTimeout::MAX)
    };

    match poll(&mut poll_fds, timeout) {
        Ok(0) => Ok(false), 
        Ok(_) => Ok(true),  
        Err(e) => Err(format!("Poll error: {}", e)),
    }
}

#[cfg(not(unix))]
fn poll_read_ready(_fd: Option<i32>, _timeout_ms: i32) -> Result<bool, String> {
    Ok(true)
}

fn run_codex_status_via_pty(command: &str, timeout: Duration) -> Result<String, String> {
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
        .map_err(|e| format!("Failed to spawn Codex: {}", e))?;

    drop(pair.slave);

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

    let needs_cursor_response = Arc::new(Mutex::new(false));
    let needs_cursor_clone = needs_cursor_response.clone();

    let stop_reader = Arc::new(AtomicBool::new(false));
    let stop_reader_clone = stop_reader.clone();

    let read_poll_timeout_ms = 500i32;

    let reader_handle = std::thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 1024];
        loop {

            if stop_reader_clone.load(Ordering::Relaxed) {
                break;
            }

            match poll_read_ready(master_fd, read_poll_timeout_ms) {
                Ok(false) => {

                    continue;
                }
                Ok(true) => {

                }
                Err(_) => {

                    break;
                }
            }

            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let data = &chunk[..n];
                    buffer_clone.lock().unwrap().extend_from_slice(data);

                    let text = String::from_utf8_lossy(data);
                    if text.contains("\x1b[6n") || text.contains("[6n") {
                        *needs_cursor_clone.lock().unwrap() = true;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let respond_to_cursor_query = |w: &mut Box<dyn Write + Send>, flag: &Arc<Mutex<bool>>| {
        let mut needs_response = flag.lock().unwrap();
        if *needs_response {

            let _ = w.write_all(b"\x1b[1;1R");
            *needs_response = false;
        }
    };

    let start = Instant::now();
    let prompt_timeout = Duration::from_secs(15);

    loop {
        if start.elapsed() > prompt_timeout {
            let _ = child.kill();
            stop_reader.store(true, Ordering::Relaxed);
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();
            return Err("Timeout waiting for Codex CLI prompt".to_string());
        }

        respond_to_cursor_query(&mut writer, &needs_cursor_response);

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        let len = data.len();
        drop(data);

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

    let status_start_offset = output_buffer.lock().unwrap().len();

    for c in "/status".chars() {
        writer.write_all(&[c as u8]).map_err(|e| format!("Failed to send: {}", e))?;
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(200));
    writer.write_all(b"\r").map_err(|e| format!("Failed to send Enter: {}", e))?;

    let status_start = Instant::now();
    let status_timeout = Duration::from_secs(10);
    let idle_threshold = Duration::from_millis(500);

    let mut last_len = status_start_offset;
    let mut last_change = Instant::now();
    let mut found_status_markers = false;

    loop {
        if status_start.elapsed() > status_timeout {
            break;
        }

        respond_to_cursor_query(&mut writer, &needs_cursor_response);

        let current_len = output_buffer.lock().unwrap().len();

        if current_len != last_len {
            last_len = current_len;
            last_change = Instant::now();

            let data = output_buffer.lock().unwrap();
            let status_slice = if status_start_offset < data.len() {
                &data[status_start_offset..]
            } else {
                &[]
            };
            let text = String::from_utf8_lossy(status_slice);
            let stripped = strip_ansi_codes(&text);
            drop(data);

            if stripped.contains("Weekly limit")
                || stripped.contains("weekly limit")
                || stripped.contains("5h limit")
                || stripped.contains("5-hour limit")
                || stripped.contains("hit your usage limit")
                || stripped.contains("data not available")
            {
                found_status_markers = true;
            }
        }

        if found_status_markers && last_change.elapsed() > idle_threshold {
            break;
        }

        if last_change.elapsed() > idle_threshold * 2 {
            break;
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = writer.write_all(&[0x03]); 
    std::thread::sleep(Duration::from_millis(200));

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

    stop_reader.store(true, Ordering::Relaxed);
    drop(writer);
    drop(pair.master);
    let _ = reader_handle.join();

    let output = output_buffer.lock().unwrap().clone();
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn strip_ansi_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some(&'[') => {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(&']') => {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next == '\x07' || next == '\\' {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn normalize_status_output(text: &str) -> String {
    let stripped = strip_ansi_codes(text);

    let without_cr = stripped.replace('\r', "");

    let lines: Vec<&str> = without_cr
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();

    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseResult {

    Success {
        hourly: Option<u8>,
        weekly: Option<u8>,
        plan: Option<String>,
    },

    UsageLimitHit,

    DataNotAvailable,

    UnrecognizedFormat,
}

pub fn parse_codex_usage(text: &str) -> ParseResult {
    let text_lower = text.to_lowercase();

    if text_lower.contains("hit your usage limit")
        || text_lower.contains("you've hit your")
        || text_lower.contains("usage limit reached")
    {
        return ParseResult::UsageLimitHit;
    }

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

        let is_hourly_limit = line_lower.contains("5h limit")
            || line_lower.contains("5-hour limit")
            || line_lower.contains("5 hour limit")
            || line_lower.contains("5 h limit")
            || line_lower.contains("hourly limit");

        if is_hourly_limit {
            if let Some(pct) = extract_percentage(line) {
                hourly = Some(pct);
            }
        }

        if line_lower.contains("weekly limit") {
            if let Some(pct) = extract_percentage(line) {
                weekly = Some(pct);
            }
        }

        if line_lower.contains("account:") {
            if let Some(start) = line.rfind('(') {
                if let Some(end) = line.rfind(')') {
                    if end > start {
                        let plan_str = &line[start + 1..end];
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

fn extract_percentage(line: &str) -> Option<u8> {
    let line_lower = line.to_lowercase();

    let patterns = [
        ("% left", false),
        ("% remaining", false),
        ("% used", true),
    ];

    for (pattern, is_used) in patterns {
        if let Some(pos) = line_lower.find(pattern) {
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
                if let Ok(pct) = digits.parse::<u8>() {
                    return if is_used {
                        Some(100u8.saturating_sub(pct))
                    } else {
                        Some(pct)
                    };
                }
            }
        }
    }
    None
}

#[allow(dead_code)]
fn extract_percentage_left(line: &str) -> Option<u8> {
    if let Some(left_pos) = line.find("% left") {
        let before = &line[..left_pos];
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

    #[test]
    fn test_extract_percentage_new() {
        assert_eq!(extract_percentage("84% left"), Some(84));
        assert_eq!(extract_percentage("[███] 100% left (resets)"), Some(100));
        assert_eq!(extract_percentage("no percentage"), None);

        assert_eq!(extract_percentage("75% remaining"), Some(75));
        assert_eq!(extract_percentage("[███] 50% remaining (resets tomorrow)"), Some(50));

        assert_eq!(extract_percentage("25% used"), Some(75));
        assert_eq!(extract_percentage("[███] 10% used (resets 12:00)"), Some(90));
        assert_eq!(extract_percentage("100% used"), Some(0));
        assert_eq!(extract_percentage("0% used"), Some(100));
    }

    #[test]
    fn test_parse_codex_usage_5hour_variations() {
        let output1 = "5-hour limit: [███] 70% left (resets 12:00)";
        let result1 = parse_codex_usage(output1);
        assert_eq!(
            result1,
            ParseResult::Success {
                hourly: Some(70),
                weekly: None,
                plan: None,
            }
        );

        let output2 = "5 hour limit: [███] 60% remaining";
        let result2 = parse_codex_usage(output2);
        assert_eq!(
            result2,
            ParseResult::Success {
                hourly: Some(60),
                weekly: None,
                plan: None,
            }
        );

        let output3 = "hourly limit: [███] 50% left";
        let result3 = parse_codex_usage(output3);
        assert_eq!(
            result3,
            ParseResult::Success {
                hourly: Some(50),
                weekly: None,
                plan: None,
            }
        );
    }

    #[test]
    fn test_parse_codex_usage_percent_used() {
        let output = r#"
  5h limit:         [██░░░░░░░░░░░░░░░░░░] 20% used
  Weekly limit:     [████░░░░░░░░░░░░░░░░] 35% used
  Account:          user@example.com (Pro)
"#;
        let result = parse_codex_usage(output);
        assert_eq!(
            result,
            ParseResult::Success {
                hourly: Some(80),
                weekly: Some(65),
                plan: Some("Pro".to_string()),
            }
        );
    }

    #[test]
    fn test_normalize_status_output() {
        let input = "line1\r\nline2\r\n\r\nline3";
        let normalized = normalize_status_output(input);
        assert_eq!(normalized, "line1\nline2\nline3");

        let with_empty = "  line1  \n\n  line2  \n  ";
        let normalized2 = normalize_status_output(with_empty);
        assert_eq!(normalized2, "line1\nline2");
    }

    #[test]
    fn test_strip_ansi_codes_osc() {
        let with_osc = "text\x1b]0;window title\x07more text";
        let stripped = strip_ansi_codes(with_osc);
        assert_eq!(stripped, "textmore text");

        let with_csi = "text\x1b[32mgreen\x1b[0m plain";
        let stripped2 = strip_ansi_codes(with_csi);
        assert_eq!(stripped2, "textgreen plain");
    }

    #[test]
    fn test_parse_with_box_drawing() {
        let output = r#"
╭──────────────────────────────────────────────────────────────╮
│  5h limit:         [█████████████████░░░] 84% left           │
│  Weekly limit:     [██████████████████░░] 89% left           │
╰──────────────────────────────────────────────────────────────╯
"#;
        let result = parse_codex_usage(output);
        assert_eq!(
            result,
            ParseResult::Success {
                hourly: Some(84),
                weekly: Some(89),
                plan: None,
            }
        );
    }

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
