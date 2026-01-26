//! Credential file reading for all providers.

use super::types::ProviderCredentials;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const CLAUDE_EMAIL_CACHE_FILE: &str = "claude_email_cache.json";

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

    // Claude: Fetch email from profile API
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

        // Fetch email from Claude profile API
        let email = if token_valid {
            fetch_claude_email(&access_token).unwrap_or_default()
        } else {
            String::new()
        };

        results.push(CredentialInfo {
            provider: "claude".to_string(),
            email,
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

/// Fetches Claude email, using cache to avoid repeated API calls.
/// Cache is keyed by refresh token hash (more stable than access token).
fn fetch_claude_email(access_token: &str) -> Option<String> {
    // Use refresh token hash as cache key (survives access token refreshes)
    let cache_key = get_claude_refresh_token_hash().unwrap_or_else(|| hash_token(access_token));

    // Try cache first
    if let Some(email) = get_cached_claude_email(&cache_key) {
        return Some(email);
    }

    // Fetch from API
    let email = fetch_claude_email_from_api(access_token)?;

    // Cache for future use
    cache_claude_email(&cache_key, &email);

    Some(email)
}

/// Get hash of Claude refresh token (more stable cache key).
fn get_claude_refresh_token_hash() -> Option<String> {
    let creds_path = claude_credentials_path().ok()?;
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let refresh_token = json["claudeAiOauth"]["refreshToken"].as_str()?;
    Some(hash_token(refresh_token))
}

/// Hash a token for use as cache key.
fn hash_token(token: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Get cached Claude email by token hash.
fn get_cached_claude_email(token_hash: &str) -> Option<String> {
    let cache_path = crate::planning_paths::planning_agent_home_dir()
        .ok()?
        .join(CLAUDE_EMAIL_CACHE_FILE);

    let content = std::fs::read_to_string(&cache_path).ok()?;
    let cache: HashMap<String, String> = serde_json::from_str(&content).ok()?;
    cache.get(token_hash).cloned()
}

/// Cache Claude email by token hash.
fn cache_claude_email(token_hash: &str, email: &str) {
    let Ok(home) = crate::planning_paths::planning_agent_home_dir() else {
        return;
    };
    let cache_path = home.join(CLAUDE_EMAIL_CACHE_FILE);

    // Load existing cache or create new
    let mut cache: HashMap<String, String> = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();

    cache.insert(token_hash.to_string(), email.to_string());

    // Save cache (ignore errors - it's just a cache)
    if let Ok(content) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(&cache_path, content);
    }
}

/// Fetch Claude email from profile API.
fn fetch_claude_email_from_api(access_token: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();

    let body: String = agent
        .get("https://api.anthropic.com/api/oauth/profile")
        .header("Authorization", &format!("Bearer {}", access_token))
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;

    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    json["account"]["email"].as_str().map(String::from)
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
#[path = "tests/credentials_tests.rs"]
mod tests;
