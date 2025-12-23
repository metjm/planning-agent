use portable_pty::{native_pty_system, CommandBuilder, PtySize};
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

fn run_claude_usage_via_pty(command: &str, timeout: Duration) -> Result<String, String> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120, 
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to allocate PTY: {}", e))?;

    let cmd = CommandBuilder::new(command);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn Claude: {}", e))?;

    drop(pair.slave);

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

    let start = Instant::now();
    let prompt_timeout = Duration::from_secs(10);

    loop {
        if start.elapsed() > prompt_timeout {
            let _ = child.kill();
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();
            return Err("Timeout waiting for Claude CLI prompt".to_string());
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        let len = data.len();
        drop(data);

        let has_prompt = stripped.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.ends_with('>') || trimmed.contains('>')
        });

        if has_prompt && len > 100 {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if start.elapsed() > timeout {
        let _ = child.kill();
        drop(writer);
        drop(pair.master);
        let _ = reader_handle.join();
        return Err("Timeout waiting for Claude CLI".to_string());
    }

    for c in "/usage".chars() {
        writer.write_all(&[c as u8]).map_err(|e| format!("Failed to send char: {}", e))?;
        std::thread::sleep(Duration::from_millis(50));
    }

    std::thread::sleep(Duration::from_millis(200));

    writer.write_all(b"\r").map_err(|e| format!("Failed to send Enter: {}", e))?;

    let usage_start = Instant::now();
    let usage_timeout = Duration::from_secs(8);
    let mut last_len = 0;

    loop {
        if usage_start.elapsed() > usage_timeout {
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

            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

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
}
