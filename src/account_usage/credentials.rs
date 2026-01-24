//! Credential file reading for all providers.

use super::types::ProviderCredentials;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Reads Claude credentials from ~/.claude/.credentials.json
pub fn read_claude_credentials() -> Result<Option<ProviderCredentials>> {
    let creds_path = claude_credentials_path()?;
    if !creds_path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&creds_path).context("Failed to read Claude credentials")?;
    let json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse Claude credentials")?;

    let oauth = &json["claudeAiOauth"];
    if oauth.is_null() {
        return Ok(None);
    }

    let access_token = oauth["accessToken"]
        .as_str()
        .context("Missing accessToken")?
        .to_string();
    let expires_at = oauth["expiresAt"].as_i64();

    Ok(Some(ProviderCredentials::Claude {
        access_token,
        expires_at,
    }))
}

fn claude_credentials_path() -> Result<PathBuf> {
    let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
        .context("Cannot determine Claude config directory")?;
    Ok(config_dir.join(".credentials.json"))
}

/// Reads Gemini credentials from ~/.gemini/oauth_creds.json
pub fn read_gemini_credentials() -> Result<Option<ProviderCredentials>> {
    let creds_path = gemini_credentials_path()?;
    if !creds_path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&creds_path).context("Failed to read Gemini credentials")?;
    let json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse Gemini credentials")?;

    let access_token = json["access_token"]
        .as_str()
        .context("Missing access_token")?
        .to_string();

    let expires_at = json["expiry_date"].as_i64();

    Ok(Some(ProviderCredentials::Gemini {
        access_token,
        expires_at,
    }))
}

fn gemini_credentials_path() -> Result<PathBuf> {
    let config_dir = std::env::var("GEMINI_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".gemini")))
        .context("Cannot determine Gemini config directory")?;
    Ok(config_dir.join("oauth_creds.json"))
}

/// Reads Codex credentials from ~/.codex/auth.json
pub fn read_codex_credentials() -> Result<Option<ProviderCredentials>> {
    let creds_path = codex_credentials_path()?;
    if !creds_path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&creds_path).context("Failed to read Codex credentials")?;
    let json: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse Codex credentials")?;

    let tokens = &json["tokens"];
    if tokens.is_null() {
        return Ok(None);
    }

    let access_token = tokens["access_token"]
        .as_str()
        .context("Missing access_token")?
        .to_string();

    let account_id = tokens["account_id"]
        .as_str()
        .context("Missing account_id")?
        .to_string();

    Ok(Some(ProviderCredentials::Codex {
        access_token,
        account_id,
    }))
}

fn codex_credentials_path() -> Result<PathBuf> {
    let config_dir = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .context("Cannot determine Codex config directory")?;
    Ok(config_dir.join("auth.json"))
}

/// Extracts email from a JWT token (for Gemini id_token or Codex access_token).
pub fn extract_email_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    // Decode payload (second part) with URL-safe base64
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;

    // Try common email claim locations
    json["email"]
        .as_str()
        .or_else(|| json["https://api.openai.com/profile"]["email"].as_str())
        .map(String::from)
}

/// Reads all available credentials from all providers.
pub fn read_all_credentials() -> Vec<(String, ProviderCredentials)> {
    let mut results = Vec::new();

    if let Ok(Some(creds)) = read_claude_credentials() {
        results.push(("claude".to_string(), creds));
    }

    if let Ok(Some(creds)) = read_gemini_credentials() {
        results.push(("gemini".to_string(), creds));
    }

    if let Ok(Some(creds)) = read_codex_credentials() {
        results.push(("codex".to_string(), creds));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_email_from_jwt_invalid() {
        assert_eq!(extract_email_from_jwt("not.a.jwt"), None);
        assert_eq!(extract_email_from_jwt("invalid"), None);
    }

    #[test]
    fn test_extract_email_from_jwt_valid() {
        // Create a simple JWT-like structure with email in payload
        // Header: {"alg":"none"}
        // Payload: {"email":"test@example.com"}
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"email":"test@example.com"}"#);
        let token = format!("{}.{}.sig", header, payload);

        assert_eq!(
            extract_email_from_jwt(&token),
            Some("test@example.com".to_string())
        );
    }
}
