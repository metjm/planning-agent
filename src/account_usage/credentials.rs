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

/// Returns credential file paths for all providers (for file watching).
pub fn credential_file_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = claude_credentials_path() {
        paths.push(path);
    }
    if let Ok(path) = gemini_credentials_path() {
        paths.push(path);
    }
    if let Ok(path) = codex_credentials_path() {
        paths.push(path);
    }
    paths
}

/// Reads all credentials and converts to CredentialInfo for RPC reporting.
/// This extracts email from tokens where possible and includes tokens for API calls.
pub fn read_all_credential_info() -> Vec<crate::rpc::host_service::CredentialInfo> {
    use crate::rpc::host_service::CredentialInfo;

    let mut results = Vec::new();

    // Claude: We don't have email without API call, use placeholder
    if let Ok(Some(ProviderCredentials::Claude {
        access_token,
        expires_at,
    })) = read_claude_credentials()
    {
        // Check if token is expired
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let token_valid = expires_at.map(|exp| exp > now_ms).unwrap_or(true);

        results.push(CredentialInfo {
            provider: "claude".to_string(),
            email: "".to_string(), // Email fetched by host via API
            token_valid,
            expires_at,
            access_token,
            account_id: None,
        });
    }

    // Gemini: Email available from id_token if present
    if let Ok(Some(ProviderCredentials::Gemini {
        access_token,
        expires_at,
    })) = read_gemini_credentials()
    {
        // Try to get email from id_token in the credentials file
        let email = get_gemini_id_token_email().unwrap_or_default();

        // Check if token is expired
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let token_valid = expires_at.map(|exp| exp > now_ms).unwrap_or(true);

        // Also check if access_token looks valid (not empty)
        let token_valid = token_valid && !access_token.is_empty();

        results.push(CredentialInfo {
            provider: "gemini".to_string(),
            email,
            token_valid,
            expires_at,
            access_token,
            account_id: None,
        });
    }

    // Codex: Email available from JWT access_token
    if let Ok(Some(ProviderCredentials::Codex {
        access_token,
        account_id,
    })) = read_codex_credentials()
    {
        let email = extract_email_from_jwt(&access_token).unwrap_or_default();

        // Check JWT expiry from exp claim
        let (token_valid, expires_at) = check_jwt_expiry(&access_token);

        results.push(CredentialInfo {
            provider: "codex".to_string(),
            email,
            token_valid,
            expires_at,
            access_token,
            account_id: Some(account_id),
        });
    }

    results
}

/// Gets email from Gemini id_token in the credentials file.
fn get_gemini_id_token_email() -> Option<String> {
    let creds_path = gemini_credentials_path().ok()?;
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let id_token = json["id_token"].as_str()?;
    extract_email_from_jwt(id_token)
}

/// Check JWT expiry and extract expires_at from exp claim.
fn check_jwt_expiry(token: &str) -> (bool, Option<i64>) {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return (false, None);
    }

    use base64::Engine;
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(p) => p,
        Err(_) => return (false, None),
    };

    let json: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(j) => j,
        Err(_) => return (false, None),
    };

    let exp = json["exp"].as_i64();
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Convert exp (seconds) to milliseconds for consistency
    let expires_at_ms = exp.map(|e| e * 1000);
    let token_valid = exp.map(|e| e > now_secs).unwrap_or(true);

    (token_valid, expires_at_ms)
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

/// Integration tests that run against real credentials on the machine.
/// These tests verify the actual credential reading and info extraction works.
#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_real_credential_file_paths() {
        let paths = credential_file_paths();
        // Should return 3 paths (claude, gemini, codex)
        assert_eq!(paths.len(), 3, "Should have 3 credential file paths");

        // Verify paths look reasonable
        for path in &paths {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(".claude")
                    || path_str.contains(".gemini")
                    || path_str.contains(".codex"),
                "Path should be for a known provider: {}",
                path_str
            );
        }
    }

    #[test]
    fn test_real_read_all_credentials() {
        // This test reads actual credential files if they exist
        let creds = read_all_credentials();

        // Print what we found for debugging
        eprintln!("Found {} credential sets", creds.len());
        for (provider, _cred) in &creds {
            eprintln!("  - {}", provider);
        }

        // If we're on a machine with credentials, verify we got them
        // Check if any of the credential files exist
        let paths = credential_file_paths();
        let existing_files: Vec<_> = paths.iter().filter(|p| p.exists()).collect();

        if !existing_files.is_empty() {
            assert!(
                !creds.is_empty(),
                "Should find credentials when credential files exist"
            );
        }
    }

    #[test]
    fn test_real_read_all_credential_info() {
        // This test reads actual credentials and converts to CredentialInfo
        let infos = read_all_credential_info();

        eprintln!("Found {} credential infos", infos.len());
        for info in &infos {
            eprintln!("  Provider: {}", info.provider);
            eprintln!(
                "    Email: {}",
                if info.email.is_empty() {
                    "(empty - fetched by host)"
                } else {
                    &info.email
                }
            );
            eprintln!("    Token valid: {}", info.token_valid);
            eprintln!("    Expires at: {:?}", info.expires_at);
        }

        // Check if any of the credential files exist
        let paths = credential_file_paths();
        let existing_files: Vec<_> = paths.iter().filter(|p| p.exists()).collect();

        if !existing_files.is_empty() {
            assert!(
                !infos.is_empty(),
                "Should find credential infos when credential files exist"
            );

            // Verify each info has required fields
            for info in &infos {
                assert!(!info.provider.is_empty(), "Provider should not be empty");
                // Email may be empty for Claude (fetched via API)
                // but Codex should have email from JWT
                if info.provider == "codex" {
                    assert!(!info.email.is_empty(), "Codex should have email from JWT");
                }
            }
        }
    }
}
