use std::process::Command;
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

    pub fn expect_not_available() -> Self {
        Self {
            error_message: Some("'expect' not installed".to_string()),
            ..Default::default()
        }
    }

    pub fn claude_not_available() -> Self {
        Self {
            error_message: Some("Claude CLI not found".to_string()),
            ..Default::default()
        }
    }
}

/// Check if the `expect` utility is available
pub fn is_expect_available() -> bool {
    Command::new("which")
        .arg("expect")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if the `claude` CLI is available
pub fn is_claude_available() -> bool {
    Command::new("which")
        .arg("claude")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Fetch Claude usage by running /usage command via expect
pub fn fetch_claude_usage_sync() -> ClaudeUsage {
    if !is_expect_available() {
        return ClaudeUsage::expect_not_available();
    }

    if !is_claude_available() {
        return ClaudeUsage::claude_not_available();
    }

    let expect_script = r#"
        spawn claude
        expect -timeout 3 ">"
        send "/usage\r"
        expect -timeout 5 eof
    "#;

    let output = match Command::new("expect")
        .args(["-c", expect_script])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return ClaudeUsage {
                error_message: Some(format!("Failed to run expect: {}", e)),
                ..Default::default()
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    // Parse usage percentages and plan info
    let weekly = parse_usage_percent(&combined, "week");
    let session = parse_usage_percent(&combined, "session")
        .or_else(|| parse_usage_percent(&combined, "daily"));
    let plan = parse_plan_type(&combined);

    ClaudeUsage {
        weekly_remaining: weekly,
        session_remaining: session,
        plan_type: plan,
        fetched_at: Some(Instant::now()),
        error_message: None,
    }
}

/// Parse percentage from text, looking for patterns like "80%" near a keyword
fn parse_usage_percent(text: &str, keyword: &str) -> Option<u8> {
    for line in text.lines() {
        let line_lower = line.to_lowercase();
        if line_lower.contains(keyword) {
            if let Some(pos) = line.find('%') {
                let before = &line[..pos];
                let digits: String = before.chars().rev()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .chars().rev().collect();
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
        assert_eq!(parse_usage_percent("Session usage: 25%", "session"), Some(25));
        assert_eq!(parse_usage_percent("Daily usage: 100%", "daily"), Some(100));
        assert_eq!(parse_usage_percent("No percentage here", "week"), None);
    }

    #[test]
    fn test_parse_plan_type() {
        assert_eq!(parse_plan_type("Plan: Max"), Some("Max".to_string()));
        assert_eq!(parse_plan_type("Your plan: Pro tier"), Some("Pro".to_string()));
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
}
