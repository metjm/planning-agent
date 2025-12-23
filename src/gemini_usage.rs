//! Gemini CLI usage tracking via /stats command

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct GeminiUsage {
    /// Usage remaining as percentage (lowest across all models)
    pub usage_remaining: Option<u8>,
    /// Model with the lowest usage remaining
    pub constrained_model: Option<String>,
    /// When this data was fetched
    pub fetched_at: Option<Instant>,
    /// Error message if fetch failed
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

/// Check if Gemini CLI is available
pub fn is_gemini_available() -> bool {
    which::which("gemini").is_ok()
}

/// Fetch Gemini usage by running /stats command via PTY
pub fn fetch_gemini_usage_sync() -> GeminiUsage {
    if !is_gemini_available() {
        return GeminiUsage::not_available();
    }

    let timeout = Duration::from_secs(20);

    match run_gemini_stats_via_pty("gemini", timeout) {
        Ok(raw_output) => {
            let output = strip_ansi_codes(&raw_output);

            // Parse usage from /stats output
            let (usage_remaining, constrained_model) = parse_gemini_usage(&output);

            GeminiUsage {
                usage_remaining,
                constrained_model,
                fetched_at: Some(Instant::now()),
                error_message: None,
            }
        }
        Err(e) => GeminiUsage::with_error(e),
    }
}

/// Run Gemini CLI and execute /stats command
fn run_gemini_stats_via_pty(command: &str, timeout: Duration) -> Result<String, String> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 150,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Failed to allocate PTY: {}", e))?;

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
    let prompt_timeout = Duration::from_secs(15);

    // Wait for Gemini prompt (look for ">" or input box)
    loop {
        if start.elapsed() > prompt_timeout {
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

        // Look for prompt indicator
        let has_prompt = stripped.contains("Type your message") || stripped.contains(">>>");

        if has_prompt && len > 500 {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    if start.elapsed() > timeout {
        let _ = child.kill();
        drop(writer);
        drop(pair.master);
        let _ = reader_handle.join();
        return Err("Timeout".to_string());
    }

    // Send /stats command
    for c in "/stats".chars() {
        writer.write_all(&[c as u8]).map_err(|e| format!("Failed to send: {}", e))?;
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(200));
    writer.write_all(b"\r").map_err(|e| format!("Failed to send Enter: {}", e))?;

    // Wait for stats output
    let stats_start = Instant::now();
    let stats_timeout = Duration::from_secs(5);

    loop {
        if stats_start.elapsed() > stats_timeout {
            break;
        }

        let data = output_buffer.lock().unwrap();
        let text = String::from_utf8_lossy(&data);
        let stripped = strip_ansi_codes(&text);
        drop(data);

        // Check if we have usage stats
        if stripped.contains("Usage left") || stripped.contains("Model Usage") {
            std::thread::sleep(Duration::from_millis(500)); // Let it finish
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // Exit
    for c in "/quit".chars() {
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

/// Parse Gemini usage from /stats output
/// Looks for lines like: "gemini-2.5-flash    -   99.3% (Resets in 23h 18m)"
fn parse_gemini_usage(text: &str) -> (Option<u8>, Option<String>) {
    let mut lowest_usage: Option<f32> = None;
    let mut constrained_model: Option<String> = None;

    for line in text.lines() {
        // Look for lines with "%" and "Resets"
        if line.contains('%') && line.contains("Resets") {
            // Try to extract model name and percentage
            // Format: "gemini-2.5-flash    -   99.3% (Resets in 23h 18m)"
            let parts: Vec<&str> = line.split_whitespace().collect();

            // Find the model name (starts with "gemini")
            let model_name = parts.iter().find(|p| p.starts_with("gemini")).map(|s| s.to_string());

            // Find the percentage
            for part in parts.iter() {
                if part.ends_with('%') {
                    let pct_str = part.trim_end_matches('%');
                    if let Ok(pct) = pct_str.parse::<f32>() {
                        // Check if this is lower than current lowest
                        if lowest_usage.is_none() || pct < lowest_usage.unwrap() {
                            lowest_usage = Some(pct);
                            constrained_model = model_name.clone();
                        }
                    }
                    break; // Only one percentage per line
                }
            }
        }
    }

    (lowest_usage.map(|p| p.round() as u8), constrained_model)
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
        let (usage, model) = parse_gemini_usage(output);
        assert_eq!(usage, Some(99)); // 99.3 rounds to 99
        assert_eq!(model, Some("gemini-2.5-flash".to_string()));
    }

    #[test]
    fn test_parse_gemini_usage_low() {
        let output = r#"
  gemini-2.5-flash               -   50.0% (Resets in 12h)
  gemini-2.5-pro                 -   25.5% (Resets in 6h)
"#;
        let (usage, model) = parse_gemini_usage(output);
        assert_eq!(usage, Some(26)); // 25.5 rounds to 26
        assert_eq!(model, Some("gemini-2.5-pro".to_string()));
    }

    /// Integration test - run with: cargo test test_fetch_gemini_usage_real -- --ignored --nocapture
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
            assert!(usage.usage_remaining.is_some(), "Should have usage data");
            eprintln!("Usage remaining: {}%", usage.usage_remaining.unwrap());
            if let Some(ref model) = usage.constrained_model {
                eprintln!("Most constrained model: {}", model);
            }
        }
    }
}
