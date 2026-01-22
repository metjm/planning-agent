//! Slash command parsing and execution for the NamingTab input.
//!
//! Supports commands like `/update`, `/config-dangerous`, and `/config dangerous`.

use crate::config::AggregationMode;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Available slash commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Install an available update.
    Update,
    /// Configure CLI tools to bypass approvals/sandbox.
    ConfigDangerous,
    /// View and resume workflow sessions.
    Sessions,
    /// Set maximum iterations for the workflow.
    MaxIterations(u32),
    /// Set sequential (true) or parallel (false) review mode.
    Sequential(bool),
    /// Set review aggregation mode.
    Aggregation(AggregationMode),
}

/// Parse a slash command from input text.
///
/// Returns `Some((command, args))` if the input is a valid slash command,
/// or `None` if it's not a command or contains paste blocks.
pub fn parse_slash_command(input: &str) -> Option<(SlashCommand, Vec<String>)> {
    let trimmed = input.trim();

    // Must start with /
    if !trimmed.starts_with('/') {
        return None;
    }

    // Split on whitespace
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let command = parts[0];
    let args: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();

    match command {
        "/update" => Some((SlashCommand::Update, args)),
        "/config-dangerous" => Some((SlashCommand::ConfigDangerous, args)),
        "/sessions" => Some((SlashCommand::Sessions, args)),
        "/max-iterations" => {
            if args.len() != 1 {
                return None;
            }
            match args[0].parse::<u32>() {
                Ok(n) if n >= 1 => Some((SlashCommand::MaxIterations(n), vec![])),
                _ => None,
            }
        }
        "/sequential" => Some((SlashCommand::Sequential(true), vec![])),
        "/parallel" => Some((SlashCommand::Sequential(false), vec![])),
        "/aggregation" => {
            if args.len() != 1 {
                return None;
            }
            let mode = match args[0].to_lowercase().as_str() {
                "any-rejects" | "any_rejects" => AggregationMode::AnyRejects,
                "all-reject" | "all_reject" => AggregationMode::AllReject,
                "majority" => AggregationMode::Majority,
                _ => return None,
            };
            Some((SlashCommand::Aggregation(mode), vec![]))
        }
        "/config" => {
            // Check for "/config dangerous" variant
            if args.first().map(|s| s.as_str()) == Some("dangerous") {
                let remaining_args: Vec<String> = args[1..].to_vec();
                Some((SlashCommand::ConfigDangerous, remaining_args))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Result of applying dangerous defaults to a single agent config.
#[derive(Debug, Clone)]
pub struct AgentConfigResult {
    pub agent_name: String,
    pub status: ConfigStatus,
    pub details: String,
}

/// Status of a config update operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigStatus {
    Updated,
    AlreadySet,
    Error,
}

/// Result of applying dangerous defaults to all agent configs.
#[derive(Debug, Clone)]
pub struct ConfigDangerousResult {
    pub results: Vec<AgentConfigResult>,
}

impl ConfigDangerousResult {
    /// Generate a summary string for display.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push("[config-dangerous] Configuring CLI tools...".to_string());

        for result in &self.results {
            let status_icon = match result.status {
                ConfigStatus::Updated => "✓",
                ConfigStatus::AlreadySet => "○",
                ConfigStatus::Error => "✗",
            };
            lines.push(format!(
                "  {} {}: {}",
                status_icon, result.agent_name, result.details
            ));
        }

        // Add note about Gemini YOLO limitation
        let gemini_result = self.results.iter().find(|r| r.agent_name == "Gemini");
        if let Some(gr) = gemini_result {
            if gr.status != ConfigStatus::Error {
                lines.push("  Note: Gemini YOLO mode requires --yolo flag per run".to_string());
            }
        }

        lines.join("\n")
    }

    /// Check if any updates had errors.
    pub fn has_errors(&self) -> bool {
        self.results.iter().any(|r| r.status == ConfigStatus::Error)
    }
}

/// Apply dangerous defaults to all supported CLI tools.
///
/// Updates configurations for:
/// - Claude: settings.json (permissions.defaultMode) and global config (bypassPermissionsModeAccepted)
/// - Codex: config.toml (approval_policy, sandbox_mode)
/// - Gemini: settings.json (tools.sandbox, security settings)
pub fn apply_dangerous_defaults() -> ConfigDangerousResult {
    let mut results = Vec::new();

    // Get home directory
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            return ConfigDangerousResult {
                results: vec![AgentConfigResult {
                    agent_name: "All".to_string(),
                    status: ConfigStatus::Error,
                    details: "Could not determine home directory".to_string(),
                }],
            };
        }
    };

    // Update Claude config
    results.push(update_claude_config(&home));

    // Update Codex config
    results.push(update_codex_config(&home));

    // Update Gemini config
    results.push(update_gemini_config(&home));

    ConfigDangerousResult { results }
}

/// Update Claude configuration files.
fn update_claude_config(home: &std::path::Path) -> AgentConfigResult {
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".claude"));

    let settings_path = config_dir.join("settings.json");
    let global_config_path = resolve_claude_global_config(&config_dir);

    let mut updated = false;
    let mut errors = Vec::new();

    // Update settings.json for permissions.defaultMode
    match update_claude_settings(&settings_path) {
        Ok(was_updated) => {
            if was_updated {
                updated = true;
            }
        }
        Err(e) => errors.push(format!("settings: {}", e)),
    }

    // Update global config for bypassPermissionsModeAccepted
    match update_claude_global_config(&global_config_path) {
        Ok(was_updated) => {
            if was_updated {
                updated = true;
            }
        }
        Err(e) => errors.push(format!("global: {}", e)),
    }

    if !errors.is_empty() {
        AgentConfigResult {
            agent_name: "Claude".to_string(),
            status: ConfigStatus::Error,
            details: errors.join("; "),
        }
    } else if updated {
        AgentConfigResult {
            agent_name: "Claude".to_string(),
            status: ConfigStatus::Updated,
            details: "bypassPermissions enabled".to_string(),
        }
    } else {
        AgentConfigResult {
            agent_name: "Claude".to_string(),
            status: ConfigStatus::AlreadySet,
            details: "already configured".to_string(),
        }
    }
}

/// Resolve the Claude global config file path.
/// Uses .config.json if it exists, otherwise .claude.json
fn resolve_claude_global_config(config_dir: &std::path::Path) -> PathBuf {
    let config_json = config_dir.join(".config.json");
    if config_json.exists() {
        config_json
    } else {
        config_dir.join(".claude.json")
    }
}

/// Update Claude settings.json to set permissions.defaultMode = "bypassPermissions"
fn update_claude_settings(path: &std::path::Path) -> Result<bool, String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Read existing or create new
    let mut json: Value = if path.exists() {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())?
    } else {
        Value::Object(serde_json::Map::new())
    };

    // Check current value
    let current_mode = json
        .get("permissions")
        .and_then(|p| p.get("defaultMode"))
        .and_then(|m| m.as_str());

    if current_mode == Some("bypassPermissions") {
        return Ok(false); // Already set
    }

    // Set permissions.defaultMode
    let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
    let permissions = obj
        .entry("permissions")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let permissions_obj = permissions
        .as_object_mut()
        .ok_or("Invalid permissions structure")?;
    permissions_obj.insert(
        "defaultMode".to_string(),
        Value::String("bypassPermissions".to_string()),
    );

    // Write back
    let output = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(path, output).map_err(|e| e.to_string())?;

    Ok(true)
}

/// Update Claude global config to set bypassPermissionsModeAccepted = true
fn update_claude_global_config(path: &std::path::Path) -> Result<bool, String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Read existing or create new
    let mut json: Value = if path.exists() {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())?
    } else {
        Value::Object(serde_json::Map::new())
    };

    // Check current value
    let current_accepted = json
        .get("bypassPermissionsModeAccepted")
        .and_then(|v| v.as_bool());

    if current_accepted == Some(true) {
        return Ok(false); // Already set
    }

    // Set bypassPermissionsModeAccepted
    let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
    obj.insert(
        "bypassPermissionsModeAccepted".to_string(),
        Value::Bool(true),
    );

    // Write back
    let output = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(path, output).map_err(|e| e.to_string())?;

    Ok(true)
}

/// Update Codex configuration file.
fn update_codex_config(home: &std::path::Path) -> AgentConfigResult {
    let config_path = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"))
        .join("config.toml");

    match update_codex_toml(&config_path) {
        Ok(was_updated) => {
            if was_updated {
                AgentConfigResult {
                    agent_name: "Codex".to_string(),
                    status: ConfigStatus::Updated,
                    details: "approval_policy=never, sandbox=full-access".to_string(),
                }
            } else {
                AgentConfigResult {
                    agent_name: "Codex".to_string(),
                    status: ConfigStatus::AlreadySet,
                    details: "already configured".to_string(),
                }
            }
        }
        Err(e) => AgentConfigResult {
            agent_name: "Codex".to_string(),
            status: ConfigStatus::Error,
            details: e,
        },
    }
}

/// Update Codex config.toml using line-based editing to avoid TOML dependency.
/// Only modifies top-level keys before the first [section].
fn update_codex_toml(path: &std::path::Path) -> Result<bool, String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Read existing content or start fresh
    let content = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let lines: Vec<&str> = content.lines().collect();

    // Parse existing top-level values
    let mut top_level: HashMap<String, String> = HashMap::new();
    let mut section_lines: Vec<String> = Vec::new();
    let mut in_section = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = true;
        }

        if in_section {
            section_lines.push(line.to_string());
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            // Parse key = value
            if let Some((key, value)) = trimmed.split_once('=') {
                top_level.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }

    // Check if already configured
    let current_approval = top_level
        .get("approval_policy")
        .map(|s| s.trim_matches('"'));
    let current_sandbox = top_level.get("sandbox_mode").map(|s| s.trim_matches('"'));

    if current_approval == Some("never") && current_sandbox == Some("danger-full-access") {
        return Ok(false); // Already set
    }

    // Set the required values
    top_level.insert("approval_policy".to_string(), "\"never\"".to_string());
    top_level.insert(
        "sandbox_mode".to_string(),
        "\"danger-full-access\"".to_string(),
    );

    // Rebuild the file
    let mut output_lines: Vec<String> = Vec::new();

    // Write top-level keys in a consistent order
    let mut keys: Vec<&String> = top_level.keys().collect();
    keys.sort();
    for key in keys {
        output_lines.push(format!("{} = {}", key, top_level[key]));
    }

    // Add a blank line before sections if there are any
    if !section_lines.is_empty() && !output_lines.is_empty() {
        output_lines.push(String::new());
    }

    // Append section content
    output_lines.extend(section_lines);

    // Write back
    let output = output_lines.join("\n");
    std::fs::write(path, output).map_err(|e| e.to_string())?;

    Ok(true)
}

/// Update Gemini configuration file.
fn update_gemini_config(home: &std::path::Path) -> AgentConfigResult {
    let gemini_dir = std::env::var("GEMINI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".gemini"));
    let settings_path = gemini_dir.join("settings.json");

    match update_gemini_settings(&settings_path) {
        Ok(was_updated) => {
            if was_updated {
                AgentConfigResult {
                    agent_name: "Gemini".to_string(),
                    status: ConfigStatus::Updated,
                    details: "sandbox disabled, auto-accept enabled".to_string(),
                }
            } else {
                AgentConfigResult {
                    agent_name: "Gemini".to_string(),
                    status: ConfigStatus::AlreadySet,
                    details: "already configured".to_string(),
                }
            }
        }
        Err(e) => AgentConfigResult {
            agent_name: "Gemini".to_string(),
            status: ConfigStatus::Error,
            details: e,
        },
    }
}

/// Update Gemini settings.json
fn update_gemini_settings(path: &std::path::Path) -> Result<bool, String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    // Read existing or create new
    let mut json: Value = if path.exists() {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())?
    } else {
        Value::Object(serde_json::Map::new())
    };

    let mut updated = false;

    // Set tools.sandbox = false
    {
        let current = json
            .get("tools")
            .and_then(|t| t.get("sandbox"))
            .and_then(|s| s.as_bool());

        if current != Some(false) {
            let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
            let tools = obj
                .entry("tools")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            let tools_obj = tools.as_object_mut().ok_or("Invalid tools structure")?;
            tools_obj.insert("sandbox".to_string(), Value::Bool(false));
            updated = true;
        }
    }

    // Set tools.autoAccept = true
    {
        let current = json
            .get("tools")
            .and_then(|t| t.get("autoAccept"))
            .and_then(|s| s.as_bool());

        if current != Some(true) {
            let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
            let tools = obj
                .entry("tools")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            let tools_obj = tools.as_object_mut().ok_or("Invalid tools structure")?;
            tools_obj.insert("autoAccept".to_string(), Value::Bool(true));
            updated = true;
        }
    }

    // Set security.disableYoloMode = false (to allow YOLO)
    {
        let current = json
            .get("security")
            .and_then(|s| s.get("disableYoloMode"))
            .and_then(|s| s.as_bool());

        if current != Some(false) {
            let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
            let security = obj
                .entry("security")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            let security_obj = security
                .as_object_mut()
                .ok_or("Invalid security structure")?;
            security_obj.insert("disableYoloMode".to_string(), Value::Bool(false));
            updated = true;
        }
    }

    // Set security.enablePermanentToolApproval = true
    {
        let current = json
            .get("security")
            .and_then(|s| s.get("enablePermanentToolApproval"))
            .and_then(|s| s.as_bool());

        if current != Some(true) {
            let obj = json.as_object_mut().ok_or("Invalid JSON structure")?;
            let security = obj
                .entry("security")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            let security_obj = security
                .as_object_mut()
                .ok_or("Invalid security structure")?;
            security_obj.insert("enablePermanentToolApproval".to_string(), Value::Bool(true));
            updated = true;
        }
    }

    if updated {
        // Write back
        let output = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(path, output).map_err(|e| e.to_string())?;
    }

    Ok(updated)
}

#[cfg(test)]
mod tests {
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
}
