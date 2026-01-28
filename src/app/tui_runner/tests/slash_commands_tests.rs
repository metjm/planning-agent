//! Tests for slash command parsing and execution.

use super::*;

#[test]
fn test_parse_update_command() {
    assert_eq!(
        parse_slash_command("/update"),
        Some((SlashCommand::Update, vec![]))
    );
    assert_eq!(
        parse_slash_command("  /update  "),
        Some((SlashCommand::Update, vec![]))
    );
}

#[test]
fn test_parse_config_dangerous_hyphen() {
    assert_eq!(
        parse_slash_command("/config-dangerous"),
        Some((SlashCommand::ConfigDangerous, vec![]))
    );
}

#[test]
fn test_parse_config_dangerous_space() {
    assert_eq!(
        parse_slash_command("/config dangerous"),
        Some((SlashCommand::ConfigDangerous, vec![]))
    );
}

#[test]
fn test_parse_unknown_command() {
    assert_eq!(parse_slash_command("/not-a-command"), None);
    assert_eq!(parse_slash_command("/config other"), None);
}

#[test]
fn test_parse_non_command() {
    assert_eq!(parse_slash_command("hello world"), None);
    assert_eq!(parse_slash_command(""), None);
    assert_eq!(parse_slash_command("  "), None);
}

#[test]
fn test_config_result_summary() {
    let result = ConfigDangerousResult {
        results: vec![
            AgentConfigResult {
                agent_name: "Claude".to_string(),
                status: ConfigStatus::Updated,
                details: "bypassPermissions enabled".to_string(),
            },
            AgentConfigResult {
                agent_name: "Codex".to_string(),
                status: ConfigStatus::AlreadySet,
                details: "already configured".to_string(),
            },
            AgentConfigResult {
                agent_name: "Gemini".to_string(),
                status: ConfigStatus::Error,
                details: "permission denied".to_string(),
            },
        ],
    };

    let summary = result.summary();
    assert!(summary.contains("Claude"));
    assert!(summary.contains("Codex"));
    assert!(summary.contains("Gemini"));
    assert!(summary.contains("✓")); // Updated
    assert!(summary.contains("○")); // AlreadySet
    assert!(summary.contains("✗")); // Error
}

#[test]
fn test_codex_toml_update() {
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!("codex_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("config.toml");

    // Test with existing content including sections
    let initial_content = r#"model = "gpt-4"
approval_policy = "on-failure"

[projects]
path = "/some/path"
"#;
    let mut file = std::fs::File::create(&config_path).unwrap();
    file.write_all(initial_content.as_bytes()).unwrap();
    drop(file);

    let was_updated = update_codex_toml(&config_path).unwrap();
    assert!(was_updated);

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("approval_policy = \"never\""));
    assert!(content.contains("sandbox_mode = \"danger-full-access\""));
    assert!(content.contains("[projects]")); // Section preserved

    // Clean up
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_claude_settings_update() {
    let temp_dir = std::env::temp_dir().join(format!("claude_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let settings_path = temp_dir.join("settings.json");

    // Test creating new file
    let was_updated = update_claude_settings(&settings_path).unwrap();
    assert!(was_updated);

    let content = std::fs::read_to_string(&settings_path).unwrap();
    let json: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        json["permissions"]["defaultMode"].as_str(),
        Some("bypassPermissions")
    );

    // Test idempotence
    let was_updated = update_claude_settings(&settings_path).unwrap();
    assert!(!was_updated);

    // Clean up
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_gemini_settings_update() {
    let temp_dir = std::env::temp_dir().join(format!("gemini_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let settings_path = temp_dir.join("settings.json");

    // Test creating new file
    let was_updated = update_gemini_settings(&settings_path).unwrap();
    assert!(was_updated);

    let content = std::fs::read_to_string(&settings_path).unwrap();
    let json: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(json["tools"]["sandbox"].as_bool(), Some(false));
    assert_eq!(json["tools"]["autoAccept"].as_bool(), Some(true));
    assert_eq!(json["security"]["disableYoloMode"].as_bool(), Some(false));
    assert_eq!(
        json["security"]["enablePermanentToolApproval"].as_bool(),
        Some(true)
    );

    // Test idempotence
    let was_updated = update_gemini_settings(&settings_path).unwrap();
    assert!(!was_updated);

    // Clean up
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_parse_max_iterations_valid() {
    assert_eq!(
        parse_slash_command("/max-iterations 5"),
        Some((SlashCommand::MaxIterations(5), vec![]))
    );
    assert_eq!(
        parse_slash_command("/max-iterations 1"),
        Some((SlashCommand::MaxIterations(1), vec![]))
    );
    assert_eq!(
        parse_slash_command("  /max-iterations 10  "),
        Some((SlashCommand::MaxIterations(10), vec![]))
    );
}

#[test]
fn test_parse_max_iterations_invalid() {
    // Zero is not allowed
    assert_eq!(parse_slash_command("/max-iterations 0"), None);
    // Non-numeric
    assert_eq!(parse_slash_command("/max-iterations abc"), None);
    // Missing argument
    assert_eq!(parse_slash_command("/max-iterations"), None);
    // Extra argument
    assert_eq!(parse_slash_command("/max-iterations 5 extra"), None);
    // Negative (parsed as non-numeric)
    assert_eq!(parse_slash_command("/max-iterations -1"), None);
}

#[test]
fn test_parse_sequential() {
    assert_eq!(
        parse_slash_command("/sequential"),
        Some((SlashCommand::Sequential(true), vec![]))
    );
    assert_eq!(
        parse_slash_command("  /sequential  "),
        Some((SlashCommand::Sequential(true), vec![]))
    );
}

#[test]
fn test_parse_parallel() {
    assert_eq!(
        parse_slash_command("/parallel"),
        Some((SlashCommand::Sequential(false), vec![]))
    );
    assert_eq!(
        parse_slash_command("  /parallel  "),
        Some((SlashCommand::Sequential(false), vec![]))
    );
}

#[test]
fn test_parse_aggregation_any_rejects() {
    assert_eq!(
        parse_slash_command("/aggregation any-rejects"),
        Some((
            SlashCommand::Aggregation(AggregationMode::AnyRejects),
            vec![]
        ))
    );
    // Also accept underscore variant
    assert_eq!(
        parse_slash_command("/aggregation any_rejects"),
        Some((
            SlashCommand::Aggregation(AggregationMode::AnyRejects),
            vec![]
        ))
    );
}

#[test]
fn test_parse_aggregation_all_reject() {
    assert_eq!(
        parse_slash_command("/aggregation all-reject"),
        Some((
            SlashCommand::Aggregation(AggregationMode::AllReject),
            vec![]
        ))
    );
    // Also accept underscore variant
    assert_eq!(
        parse_slash_command("/aggregation all_reject"),
        Some((
            SlashCommand::Aggregation(AggregationMode::AllReject),
            vec![]
        ))
    );
}

#[test]
fn test_parse_aggregation_majority() {
    assert_eq!(
        parse_slash_command("/aggregation majority"),
        Some((SlashCommand::Aggregation(AggregationMode::Majority), vec![]))
    );
    // Case insensitive
    assert_eq!(
        parse_slash_command("/aggregation MAJORITY"),
        Some((SlashCommand::Aggregation(AggregationMode::Majority), vec![]))
    );
}

#[test]
fn test_parse_aggregation_invalid() {
    // Unknown mode
    assert_eq!(parse_slash_command("/aggregation invalid"), None);
    // Missing argument
    assert_eq!(parse_slash_command("/aggregation"), None);
    // Extra argument
    assert_eq!(parse_slash_command("/aggregation any-rejects extra"), None);
}

#[test]
fn test_parse_workflow_no_args() {
    assert_eq!(
        parse_slash_command("/workflow"),
        Some((SlashCommand::Workflow(None), vec![]))
    );
    assert_eq!(
        parse_slash_command("  /workflow  "),
        Some((SlashCommand::Workflow(None), vec![]))
    );
}

#[test]
fn test_parse_workflow_with_name() {
    assert_eq!(
        parse_slash_command("/workflow claude-only"),
        Some((
            SlashCommand::Workflow(Some("claude-only".to_string())),
            vec![]
        ))
    );
    assert_eq!(
        parse_slash_command("/workflow default"),
        Some((SlashCommand::Workflow(Some("default".to_string())), vec![]))
    );
    assert_eq!(
        parse_slash_command("/workflow my-custom"),
        Some((
            SlashCommand::Workflow(Some("my-custom".to_string())),
            vec![]
        ))
    );
}

#[test]
fn test_parse_merge_worktree() {
    assert_eq!(
        parse_slash_command("/merge-worktree"),
        Some((SlashCommand::MergeWorktree, vec![]))
    );
    assert_eq!(
        parse_slash_command("  /merge-worktree  "),
        Some((SlashCommand::MergeWorktree, vec![]))
    );
}
