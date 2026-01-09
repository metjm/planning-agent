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
    assert_eq!(parse_plan_type("Your plan: Pro tier"), Some("Pro".to_string()));
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
    assert_eq!(strip_ansi_codes("\x1b[1;31mBold Red\x1b[0m Text"), "Bold Red Text");
    assert_eq!(strip_ansi_codes("Plain text"), "Plain text");
    assert_eq!(strip_ansi_codes("\x1b[33mYellow\x1b[0m \x1b[34mBlue\x1b[0m"), "Yellow Blue");
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
        assert!(!msg.to_lowercase().contains("expect"), "Error message should not mention expect");
    }

    let claude_not_found = ClaudeUsage::claude_not_available();
    if let Some(msg) = &claude_not_found.error_message {
        assert!(!msg.to_lowercase().contains("expect"), "Error message should not mention expect");
    }
}

#[test]
fn test_claude_usage_with_error_sets_fetched_at() {
    let usage = ClaudeUsage::with_error("Test error".to_string());
    assert!(usage.fetched_at.is_some(), "with_error should set fetched_at");
    assert_eq!(usage.error_message, Some("Test error".to_string()));
}

#[test]
fn test_claude_usage_not_available_sets_fetched_at() {
    let usage = ClaudeUsage::claude_not_available();
    assert!(usage.fetched_at.is_some(), "claude_not_available should set fetched_at");
    assert_eq!(usage.error_message, Some("Claude CLI not found".to_string()));
}

#[test]
fn test_detect_cli_state_ready() {
    assert_eq!(detect_cli_state("Opus 4.5 · Claude Max · user@example.com >"), CliState::Ready);
    assert_eq!(detect_cli_state("Sonnet · Claude Pro > "), CliState::Ready);
    assert_eq!(detect_cli_state("Loading...\n>"), CliState::Ready);
}

#[test]
fn test_detect_cli_state_requires_auth() {
    assert_eq!(detect_cli_state("Please log in to continue"), CliState::RequiresAuth);
    assert_eq!(detect_cli_state("You are not logged in"), CliState::RequiresAuth);
    assert_eq!(detect_cli_state("Please authenticate first"), CliState::RequiresAuth);
    assert_eq!(detect_cli_state("API key required"), CliState::RequiresAuth);
}

#[test]
fn test_detect_cli_state_first_run() {
    // Existing reliable patterns
    assert_eq!(detect_cli_state("Welcome to Claude Code! Let's get started."), CliState::FirstRun);
    assert_eq!(detect_cli_state("First time setup required"), CliState::FirstRun);
    // New specific setup patterns
    assert_eq!(detect_cli_state("Please complete setup to continue"), CliState::FirstRun);
    assert_eq!(detect_cli_state("You need to finish setup first"), CliState::FirstRun);
    assert_eq!(detect_cli_state("Initial setup required"), CliState::FirstRun);
    assert_eq!(detect_cli_state("Setup is required to continue"), CliState::FirstRun);
    // New specific configure patterns
    assert_eq!(detect_cli_state("Configure Claude Code settings"), CliState::FirstRun);
    assert_eq!(detect_cli_state("Configuration required before use"), CliState::FirstRun);
}

#[test]
fn test_detect_cli_state_false_positive_prevention() {
    // These generic phrases should NOT trigger FirstRun detection
    // Generic "setup" in conversation context
    assert!(matches!(detect_cli_state("Let me help you setup your project"), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state("Run npm run setup"), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state("Here's how to setup your database"), CliState::Unknown(_)));
    // Generic "configure" in conversation context
    assert!(matches!(detect_cli_state("I'll configure the database for you"), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state("Here's how to configure eslint"), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state("You can configure the settings in config.json"), CliState::Unknown(_)));
    // Claude CLI welcome screen sidebar text should NOT trigger FirstRun
    // The sidebar always shows "Tips for getting started" even for configured users
    assert!(matches!(detect_cli_state("Tips for getting started"), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state("Welcome back! Tips for getting started with your project"), CliState::Unknown(_)));
}

#[test]
fn test_detect_cli_state_unknown_has_sanitized_excerpt() {
    let ansi_output = "\x1b[32mLoading spinner\x1b[0m with colors";
    let state = detect_cli_state(ansi_output);
    if let CliState::Unknown(excerpt) = state {
        assert!(!excerpt.contains("\x1b"), "Excerpt should not contain ANSI escape sequences");
        assert!(excerpt.contains("Loading spinner"), "Excerpt should contain the text content");
    } else {
        panic!("Expected CliState::Unknown");
    }
}

#[test]
fn test_detect_cli_state_unknown() {
    assert!(matches!(detect_cli_state("Loading spinner..."), CliState::Unknown(_)));
    assert!(matches!(detect_cli_state(""), CliState::Unknown(_)));
}

#[test]
fn test_detect_cli_state_unknown_truncates_unicode_safely() {
    let output = "─".repeat(120);
    let state = detect_cli_state(&output);
    if let CliState::Unknown(excerpt) = state {
        assert!(excerpt.ends_with("..."), "Excerpt should be truncated with ellipsis");
        assert_eq!(excerpt.chars().count(), 103);
    } else {
        panic!("Expected CliState::Unknown");
    }
}

#[test]
fn test_default_timeouts() {
    let prompt = get_prompt_timeout();
    assert_eq!(prompt.as_secs(), 10);
    let usage = get_usage_timeout();
    assert_eq!(usage.as_secs(), 8);
    let overall = get_overall_timeout();
    assert_eq!(overall.as_secs(), 25);
}

#[test]
fn test_is_debug_enabled_default() {
    let _ = is_debug_enabled();
}

#[test]
fn test_debug_logger_disabled() {
    let mut logger = DebugLogger::new();
    logger.log("Test message");
    logger.log_output_snapshot("Test", "some output", 100);
}

#[test]
fn test_debug_logger_output_snapshot_truncation() {
    let mut logger = DebugLogger::new();
    let long_output = "a".repeat(1000);
    logger.log_output_snapshot("Long output", &long_output, 100);
}

#[test]
fn test_debug_logger_output_snapshot_unicode_truncation() {
    let mut logger = DebugLogger {
        enabled: true,
        start: Instant::now(),
        entries: Vec::new(),
    };
    let long_output = "─".repeat(200);
    logger.log_output_snapshot("Unicode output", &long_output, 50);
    logger.enabled = false;
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
    if !is_claude_available() {
        eprintln!("Claude CLI not found, skipping integration test");
        return;
    }

    eprintln!("Fetching Claude usage with debug logging enabled...");
    eprintln!("Debug logs will be written to ~/.planning-agent/logs/claude-usage.log");

    let usage = fetch_claude_usage_sync();
    eprintln!("Result: {:?}", usage);

    if is_debug_enabled() {
        if let Ok(log_path) = planning_paths::claude_usage_log_path() {
            assert!(log_path.exists(), "Debug log file should exist when CLAUDE_USAGE_DEBUG=1");
            eprintln!("Debug log written to: {:?}", log_path);
        }
    }
}
