use crate::planning_paths;
use crate::usage_reset::{ResetTimestamp, UsageWindow, UsageWindowSpan};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct GeminiUsage {
    /// Daily usage window with reset timestamp
    pub daily: UsageWindow,

    pub fetched_at: Option<Instant>,

    pub error_message: Option<String>,
}

impl GeminiUsage {
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

pub fn is_gemini_available() -> bool {
    which::which("gemini").is_ok()
}

/// Returns true if debug logging is enabled via GEMINI_USAGE_DEBUG=1
fn is_debug_enabled() -> bool {
    env::var("GEMINI_USAGE_DEBUG")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get prompt timeout from env or use default (15 seconds)
fn get_prompt_timeout() -> Duration {
    env::var("GEMINI_USAGE_PROMPT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(15))
}

/// Get stats output timeout from env or use default (10 seconds)
fn get_stats_timeout() -> Duration {
    env::var("GEMINI_USAGE_STATS_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(10))
}

/// Get overall timeout from env or use default (30 seconds)
fn get_overall_timeout() -> Duration {
    env::var("GEMINI_USAGE_OVERALL_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
}

/// Debug logger that writes to ~/.planning-agent/logs/gemini-usage.log when GEMINI_USAGE_DEBUG=1
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

    fn flush_to_file(&self) {
        if !self.enabled || self.entries.is_empty() {
            return;
        }
        if let Ok(log_path) = planning_paths::gemini_usage_log_path() {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(file, "\n=== Gemini Usage Fetch {} ===", timestamp);
                for entry in &self.entries {
                    let _ = writeln!(file, "{}", entry);
                }
            }
        }
    }
}

impl Drop for DebugLogger {
    fn drop(&mut self) {
        self.flush_to_file();
    }
}

pub fn fetch_gemini_usage_sync() -> GeminiUsage {
    let mut logger = DebugLogger::new();
    logger.log("Starting Gemini usage fetch");

    if !is_gemini_available() {
        logger.log("Gemini CLI not available");
        return GeminiUsage::not_available();
    }

    let timeout = get_overall_timeout();
    logger.log(&format!(
        "Timeouts: prompt={}s, stats={}s, overall={}s",
        get_prompt_timeout().as_secs(),
        get_stats_timeout().as_secs(),
        timeout.as_secs()
    ));

    match run_gemini_stats_via_pty("gemini", timeout, &mut logger) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);
            logger.log(&format!("Got output ({} bytes)", output.len()));

            // Parse returns (remaining %, reset_duration) from the lowest usage line
            let (usage_remaining, reset_duration) = parse_gemini_usage_with_reset(&output);
            let daily_used = usage_remaining.map(|r| 100u8.saturating_sub(r));
            logger.log(&format!(
                "Parsed: remaining={:?}%, used={:?}%",
                usage_remaining, daily_used
            ));

            // Build usage window with reset timestamp and span
            // Gemini has daily usage windows (24h)
            let daily = match (daily_used, reset_duration) {
                (Some(pct), Some(dur)) => {
                    let reset_ts = ResetTimestamp::from_duration_from_now(dur);
                    UsageWindow::with_percent_reset_and_span(pct, reset_ts, UsageWindowSpan::Days(1))
                }
                (Some(pct), None) => UsageWindow::with_percent_and_span(pct, UsageWindowSpan::Days(1)),
                _ => UsageWindow::default(),
            };

            logger.log("Fetch completed successfully");
            GeminiUsage {
                daily,
                fetched_at: Some(Instant::now()),
                error_message: None,
            }
        }
        Err(e) => {
            logger.log(&format!("Fetch failed: {}", e));
            GeminiUsage::with_error(e)
        }
    }
}

fn run_gemini_stats_via_pty(
    command: &str,
    timeout: Duration,
    logger: &mut DebugLogger,
) -> Result<String, String> {
    logger.log("Allocating PTY");
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 150,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to allocate PTY: {}", e))?;

    logger.log("Spawning Gemini CLI");
    let cmd = CommandBuilder::new(command);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn Gemini: {}", e))?;

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
    let prompt_timeout = get_prompt_timeout();
    logger.log(&format!(
        "Waiting for CLI prompt (timeout: {}s)",
        prompt_timeout.as_secs()
    ));

    loop {
        if start.elapsed() > prompt_timeout {
            logger.log(&format!(
                "Prompt timeout after {:.1}s",
                start.elapsed().as_secs_f64()
            ));
            let _ = child.kill();
            drop(writer);
            drop(pair.master);
            let _ = reader_handle.join();
            return Err("Timeout waiting for Gemini CLI prompt".to_string());
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        let len = data.len();
        drop(data);

        let has_prompt = stripped.contains("Type your message") || stripped.contains(">>>");

        if has_prompt && len > 500 {
            logger.log(&format!(
                "Found prompt after {:.1}s ({} bytes)",
                start.elapsed().as_secs_f64(),
                len
            ));
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if start.elapsed() > timeout {
        logger.log("Overall timeout exceeded after finding prompt");
        let _ = child.kill();
        drop(writer);
        drop(pair.master);
        let _ = reader_handle.join();
        return Err("Timeout".to_string());
    }

    logger.log("Sending /stats command");
    for c in "/stats".chars() {
        writer
            .write_all(&[c as u8])
            .map_err(|e| format!("Failed to send: {}", e))?;
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(200));
    writer
        .write_all(b"\r")
        .map_err(|e| format!("Failed to send Enter: {}", e))?;

    let stats_start = Instant::now();
    let stats_timeout = get_stats_timeout();
    logger.log(&format!(
        "Waiting for stats output (timeout: {}s)",
        stats_timeout.as_secs()
    ));

    let mut stats_found = false;
    loop {
        if stats_start.elapsed() > stats_timeout {
            logger.log(&format!(
                "Stats timeout after {:.1}s (no usage data found)",
                stats_start.elapsed().as_secs_f64()
            ));
            break;
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        drop(data);

        if stripped.contains("Usage left") || stripped.contains("Model Usage") {
            logger.log(&format!(
                "Found stats output after {:.1}s",
                stats_start.elapsed().as_secs_f64()
            ));
            stats_found = true;
            std::thread::sleep(Duration::from_millis(500));
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if !stats_found {
        logger.log("Warning: Stats not found within timeout, proceeding with available output");
    }

    logger.log("Sending /quit command");
    for c in "/quit".chars() {
        let _ = writer.write_all(&[c as u8]);
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = writer.write_all(b"\r");

    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                logger.log("Process exited cleanly");
                break;
            }
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    logger.log("Exit timeout, killing process");
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                logger.log("Error checking process status, killing");
                let _ = child.kill();
                break;
            }
        }
    }

    drop(writer);
    drop(pair.master);
    let _ = reader_handle.join();

    let output = output_buffer.lock().unwrap().clone();
    logger.log(&format!("Total output: {} bytes", output.len()));
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

/// Parse Gemini usage remaining percentage (for backwards compatibility in tests)
#[cfg(test)]
fn parse_gemini_usage(text: &str) -> Option<u8> {
    let (usage, _) = parse_gemini_usage_with_reset(text);
    usage
}

/// Parse Gemini usage and reset duration from the line with lowest usage.
///
/// Returns (usage_remaining_percent, reset_duration) where:
/// - usage_remaining_percent is the lowest percentage found
/// - reset_duration is the duration parsed from that same line
fn parse_gemini_usage_with_reset(text: &str) -> (Option<u8>, Option<Duration>) {
    let mut lowest_usage: Option<f32> = None;
    let mut lowest_reset_duration: Option<Duration> = None;

    for line in text.lines() {
        if line.contains('%') && line.contains("Resets") {
            let parts: Vec<&str> = line.split_whitespace().collect();

            // Find percentage
            let mut line_pct: Option<f32> = None;
            for part in parts.iter() {
                if part.ends_with('%') {
                    let pct_str = part.trim_end_matches('%');
                    if let Ok(pct) = pct_str.parse::<f32>() {
                        line_pct = Some(pct);
                    }
                    break;
                }
            }

            // If this line has the lowest usage, capture its reset duration
            if let Some(pct) = line_pct {
                if lowest_usage.is_none() || pct < lowest_usage.unwrap() {
                    lowest_usage = Some(pct);
                    lowest_reset_duration = parse_reset_duration(line);
                }
            }
        }
    }

    (lowest_usage.map(|p| p.round() as u8), lowest_reset_duration)
}

/// Parse reset duration from a line like "... (Resets in 23h 18m)" or "... (Resets in 24h)"
fn parse_reset_duration(line: &str) -> Option<Duration> {
    // Find "Resets in" pattern
    let lower = line.to_lowercase();
    let resets_pos = lower.find("resets in ")?;
    let after_resets = &line[resets_pos + 10..];

    // Extract duration text (until ')' or end of line)
    let duration_text = if let Some(paren_pos) = after_resets.find(')') {
        after_resets[..paren_pos].trim()
    } else {
        after_resets.trim()
    };

    // Parse duration components (e.g., "23h 18m", "24h", "18m")
    let mut total_secs: u64 = 0;
    let lower_duration = duration_text.to_lowercase();

    // Parse hours
    if let Some(h_pos) = lower_duration.find('h') {
        let h_str = lower_duration[..h_pos].split_whitespace().last()?;
        if let Ok(hours) = h_str.parse::<u64>() {
            total_secs += hours * 3600;
        }
    }

    // Parse minutes
    if let Some(m_pos) = lower_duration.find('m') {
        // Find the number before 'm'
        let before_m = &lower_duration[..m_pos];
        let m_str = before_m.split_whitespace().last()?;
        // Strip 'h' suffix if present (e.g., "23h" from "23h 18m")
        let m_str = m_str.trim_end_matches('h');
        if !m_str.is_empty() {
            if let Ok(minutes) = m_str.parse::<u64>() {
                total_secs += minutes * 60;
            }
        }
    }

    if total_secs > 0 {
        Some(Duration::from_secs(total_secs))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_usage() {
        let output = r#"
  Model Usage                 Reqs                  Usage left
  ────────────────────────────────────────────────────────────
  gemini-2.5-flash               -   99.3% (Resets in 23h 18m)
  gemini-2.5-flash-lite          -   99.3% (Resets in 23h 18m)
  gemini-2.5-pro                 -      100.0% (Resets in 24h)
  gemini-3-flash-preview         -   99.9% (Resets in 23h 41m)
  gemini-3-pro-preview           -      100.0% (Resets in 24h)
"#;
        let usage = parse_gemini_usage(output);
        assert_eq!(usage, Some(99));
    }

    #[test]
    fn test_parse_gemini_usage_low() {
        let output = r#"
  gemini-2.5-flash               -   50.0% (Resets in 12h)
  gemini-2.5-pro                 -   25.5% (Resets in 6h)
"#;
        let usage = parse_gemini_usage(output);
        assert_eq!(usage, Some(26));
    }

    #[test]
    fn test_parse_reset_duration_hours_and_minutes() {
        let line = "gemini-2.5-flash - 99.3% (Resets in 23h 18m)";
        let duration = parse_reset_duration(line);
        assert_eq!(duration, Some(Duration::from_secs(23 * 3600 + 18 * 60)));
    }

    #[test]
    fn test_parse_reset_duration_hours_only() {
        let line = "gemini-2.5-pro - 100.0% (Resets in 24h)";
        let duration = parse_reset_duration(line);
        assert_eq!(duration, Some(Duration::from_secs(24 * 3600)));
    }

    #[test]
    fn test_parse_reset_duration_minutes_only() {
        let line = "gemini-2.5-flash - 99.0% (Resets in 45m)";
        let duration = parse_reset_duration(line);
        assert_eq!(duration, Some(Duration::from_secs(45 * 60)));
    }

    #[test]
    fn test_parse_reset_duration_no_match() {
        let line = "gemini-2.5-flash - 99.0%";
        let duration = parse_reset_duration(line);
        assert_eq!(duration, None);
    }

    #[test]
    fn test_parse_gemini_usage_with_reset_captures_duration() {
        let output = r#"
  gemini-2.5-flash               -   50.0% (Resets in 12h)
  gemini-2.5-pro                 -   25.5% (Resets in 6h)
"#;
        let (usage, duration) = parse_gemini_usage_with_reset(output);
        assert_eq!(usage, Some(26)); // Lowest is 25.5% -> rounds to 26%
        // The duration should be from the 25.5% line (6h)
        assert_eq!(duration, Some(Duration::from_secs(6 * 3600)));
    }

    #[test]
    #[ignore]
    fn test_fetch_gemini_usage_real() {
        if !is_gemini_available() {
            eprintln!("Gemini CLI not found, skipping");
            return;
        }

        eprintln!("Fetching real Gemini usage...");
        let usage = fetch_gemini_usage_sync();
        eprintln!("Result: {:?}", usage);

        assert!(usage.fetched_at.is_some());
        if usage.error_message.is_none() {
            assert!(
                usage.daily.used_percent.is_some(),
                "Should have daily usage data"
            );
            eprintln!("Daily used: {}%", usage.daily.used_percent.unwrap());
            if let Some(remaining) = usage.daily.time_until_reset() {
                eprintln!(
                    "Resets in: {}h {}m",
                    remaining.as_secs() / 3600,
                    (remaining.as_secs() % 3600) / 60
                );
            }
        }
    }
}
