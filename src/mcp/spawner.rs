use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use uuid::Uuid;

/// Configuration for an MCP server that can be used by any agent
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Unique server name (e.g., "planning-agent-review-{uuid}")
    pub server_name: String,
    /// Path to the executable command
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
}

impl McpServerConfig {
    /// Create a new MCP server config with a unique name
    pub fn new(plan_content: &str, review_prompt: &str) -> Result<Self> {
        let exe = std::env::current_exe()?;
        let server_name = generate_unique_server_name();

        // Encode plan and prompt as base64 to safely pass via command line
        let plan_b64 = BASE64.encode(plan_content);
        let prompt_b64 = BASE64.encode(review_prompt);

        Ok(Self {
            server_name,
            command: exe.to_string_lossy().to_string(),
            args: vec![
                "--internal-mcp-server".to_string(),
                "--plan-content-b64".to_string(),
                plan_b64,
                "--review-prompt-b64".to_string(),
                prompt_b64,
            ],
        })
    }

    /// Generate Claude-compatible MCP config JSON (used with --mcp-config flag)
    pub fn to_claude_json(&self) -> String {
        let config = json!({
            "mcpServers": {
                &self.server_name: {
                    "command": &self.command,
                    "args": &self.args
                }
            }
        });
        config.to_string()
    }

    /// Generate Codex CLI --config arguments for MCP server injection
    /// Returns a vector of argument pairs to prepend to the command
    pub fn to_codex_config_args(&self) -> Vec<String> {
        let args_json = serde_json::to_string(&self.args).unwrap_or_else(|_| "[]".to_string());

        vec![
            "--config".to_string(),
            format!("mcp_servers.{}.command={}", self.server_name, self.command),
            "--config".to_string(),
            format!("mcp_servers.{}.args={}", self.server_name, args_json),
        ]
    }

    /// Generate Gemini settings.json content for MCP server
    pub fn to_gemini_settings_json(&self) -> String {
        let config = json!({
            "mcpServers": {
                &self.server_name: {
                    "command": &self.command,
                    "args": &self.args
                }
            }
        });
        serde_json::to_string_pretty(&config).unwrap_or_else(|_| config.to_string())
    }
}

/// Generate a unique MCP server name to prevent collisions
pub fn generate_unique_server_name() -> String {
    format!("planning-agent-review-{}", Uuid::new_v4())
}

/// Generate MCP config JSON for Claude without spawning a subprocess.
/// Claude will spawn the MCP server itself using this config.
/// Returns both the config JSON and the unique server name.
#[allow(dead_code)]
pub fn generate_mcp_config(plan_content: &str, review_prompt: &str) -> Result<String> {
    let config = McpServerConfig::new(plan_content, review_prompt)?;
    Ok(config.to_claude_json())
}

/// Generate MCP config with the server config struct for more control
pub fn generate_mcp_server_config(plan_content: &str, review_prompt: &str) -> Result<McpServerConfig> {
    McpServerConfig::new(plan_content, review_prompt)
}

/// Decode base64 plan content
pub fn decode_plan_content(b64: &str) -> Result<String> {
    let bytes = BASE64.decode(b64)?;
    Ok(String::from_utf8(bytes)?)
}

/// Decode base64 review prompt
pub fn decode_review_prompt(b64: &str) -> Result<String> {
    let bytes = BASE64.decode(b64)?;
    Ok(String::from_utf8(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unique_server_name() {
        let name1 = generate_unique_server_name();
        let name2 = generate_unique_server_name();

        assert!(name1.starts_with("planning-agent-review-"));
        assert!(name2.starts_with("planning-agent-review-"));
        assert_ne!(name1, name2); // Should be unique
    }

    #[test]
    fn test_mcp_server_config_new() {
        let config = McpServerConfig::new("# Test Plan", "Review this").unwrap();

        assert!(config.server_name.starts_with("planning-agent-review-"));
        assert!(!config.command.is_empty());
        assert_eq!(config.args.len(), 5);
        assert_eq!(config.args[0], "--internal-mcp-server");
    }

    #[test]
    fn test_mcp_server_config_to_claude_json() {
        let config = McpServerConfig::new("# Test Plan", "Review this").unwrap();
        let json_str = config.to_claude_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Should have mcpServers with our unique server name
        let servers = parsed["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        let (server_name, server_config) = servers.iter().next().unwrap();
        assert!(server_name.starts_with("planning-agent-review-"));
        assert!(server_config["command"].as_str().is_some());
        assert!(server_config["args"].as_array().is_some());
    }

    #[test]
    fn test_mcp_server_config_to_codex_args() {
        let config = McpServerConfig::new("# Test Plan", "Review this").unwrap();
        let args = config.to_codex_config_args();

        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "--config");
        assert!(args[1].starts_with("mcp_servers.planning-agent-review-"));
        assert!(args[1].contains(".command="));
        assert_eq!(args[2], "--config");
        assert!(args[3].starts_with("mcp_servers.planning-agent-review-"));
        assert!(args[3].contains(".args="));
    }

    #[test]
    fn test_mcp_server_config_to_gemini_settings() {
        let config = McpServerConfig::new("# Test Plan", "Review this").unwrap();
        let json_str = config.to_gemini_settings_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Should have mcpServers with our unique server name
        let servers = parsed["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        let (server_name, server_config) = servers.iter().next().unwrap();
        assert!(server_name.starts_with("planning-agent-review-"));
        assert!(server_config["command"].as_str().is_some());
        assert!(server_config["args"].as_array().is_some());
    }

    #[test]
    fn test_generate_mcp_config() {
        let config = generate_mcp_config("# Test Plan", "Review this").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        // Should have mcpServers with a unique server name
        let servers = parsed["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        let (server_name, server_config) = servers.iter().next().unwrap();
        assert!(server_name.starts_with("planning-agent-review-"));
        assert!(server_config["command"].as_str().is_some());
        assert!(server_config["args"].as_array().is_some());
    }

    #[test]
    fn test_decode_plan_content() {
        let original = "# Test Plan\n\nSome content";
        let encoded = BASE64.encode(original);
        let decoded = decode_plan_content(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_decode_review_prompt() {
        let original = "Review this plan carefully";
        let encoded = BASE64.encode(original);
        let decoded = decode_review_prompt(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_base64_roundtrip_unicode() {
        let original = "# Plan\n\nIncluding unicode: \u{1F600} and more";
        let encoded = BASE64.encode(original);
        let decoded = decode_plan_content(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
