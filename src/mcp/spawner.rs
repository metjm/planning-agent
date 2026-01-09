use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;

/// Generate MCP config JSON for Claude without spawning a subprocess.
/// Claude will spawn the MCP server itself using this config.
pub fn generate_mcp_config(plan_content: &str, review_prompt: &str) -> Result<String> {
    let exe = std::env::current_exe()?;

    // Encode plan and prompt as base64 to safely pass via command line
    let plan_b64 = BASE64.encode(plan_content);
    let prompt_b64 = BASE64.encode(review_prompt);

    let config = json!({
        "mcpServers": {
            "planning-agent-review": {
                "command": exe.to_string_lossy(),
                "args": [
                    "--internal-mcp-server",
                    "--plan-content-b64", plan_b64,
                    "--review-prompt-b64", prompt_b64
                ]
            }
        }
    });

    Ok(config.to_string())
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
    fn test_generate_mcp_config() {
        let config = generate_mcp_config("# Test Plan", "Review this").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();

        assert!(parsed["mcpServers"]["planning-agent-review"].is_object());
        assert!(parsed["mcpServers"]["planning-agent-review"]["command"]
            .as_str()
            .is_some());
        assert!(parsed["mcpServers"]["planning-agent-review"]["args"]
            .as_array()
            .is_some());
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
