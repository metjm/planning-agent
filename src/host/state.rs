//! Host state management.
//!
//! Tracks connected containers and their sessions, providing aggregated
//! session data for display in the GUI.

use crate::account_usage::store::UsageStore;
use crate::account_usage::types::ProviderCredentials;
use crate::host::SessionInfo;
use crate::rpc::host_service::{AccountUsageInfo, CredentialInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Represents a connected container daemon.
#[derive(Debug, Clone)]
pub struct ConnectedContainer {
    pub container_name: String,
    /// Working directory from ContainerInfo hello handshake.
    /// Only read by GUI code (behind host-gui feature).
    #[cfg_attr(not(feature = "host-gui"), allow(dead_code))]
    pub working_dir: PathBuf,
    /// When this container connected.
    /// Only read by GUI code (behind host-gui feature).
    #[cfg_attr(not(feature = "host-gui"), allow(dead_code))]
    pub connected_at: Instant,
    pub last_message_at: Instant,
    pub sessions: HashMap<String, SessionInfo>,
    /// Git commit SHA the daemon was built from.
    pub git_sha: String,
    /// Unix timestamp when the daemon was built.
    pub build_timestamp: u64,
}

impl ConnectedContainer {
    pub fn new(
        container_name: String,
        working_dir: PathBuf,
        git_sha: String,
        build_timestamp: u64,
    ) -> Self {
        let now = Instant::now();
        Self {
            container_name,
            working_dir,
            connected_at: now,
            last_message_at: now,
            sessions: HashMap::new(),
            git_sha,
            build_timestamp,
        }
    }
}

/// Aggregated state for the host application.
pub struct HostState {
    /// Connected containers by container_id
    pub containers: HashMap<String, ConnectedContainer>,
    /// Flattened session list for display (computed on demand)
    cached_sessions: Option<Vec<DisplaySession>>,
    /// Last time any session changed (for "last update" display)
    pub last_update: Instant,
    /// Account usage tracking store
    pub usage_store: UsageStore,
    /// Credentials received from daemons, keyed by (provider, email).
    /// Supports multiple accounts per provider.
    daemon_credentials: HashMap<(String, String), ProviderCredentials>,
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

/// Session with container context for display.
#[derive(Debug, Clone)]
pub struct DisplaySession {
    pub container_name: String,
    pub session: SessionInfo,
}

impl DisplaySession {
    pub fn new(container_name: String, session: SessionInfo) -> Self {
        Self {
            container_name,
            session,
        }
    }
}

impl HostState {
    pub fn new() -> Self {
        // Try to load existing usage store, fall back to empty
        let usage_store = UsageStore::load().unwrap_or_else(|e| {
            eprintln!("[host-state] Warning: Could not load usage store: {}", e);
            UsageStore::new()
        });

        Self {
            containers: HashMap::new(),
            cached_sessions: None,
            last_update: Instant::now(),
            usage_store,
            daemon_credentials: HashMap::new(),
        }
    }

    /// Store credentials received from a daemon.
    /// Converts CredentialInfo to ProviderCredentials for API calls.
    /// Keys by (provider, identifier) to support multiple accounts per provider.
    /// For Claude where email is unknown until API call, uses token hash as identifier.
    pub fn store_credentials(&mut self, credentials: Vec<CredentialInfo>) {
        for cred in credentials {
            let provider_creds = match cred.provider.as_str() {
                "claude" => ProviderCredentials::Claude {
                    access_token: cred.access_token.clone(),
                    expires_at: cred.expires_at,
                },
                "gemini" => ProviderCredentials::Gemini {
                    access_token: cred.access_token.clone(),
                    expires_at: cred.expires_at,
                },
                "codex" => ProviderCredentials::Codex {
                    access_token: cred.access_token.clone(),
                    account_id: cred.account_id.unwrap_or_default(),
                },
                _ => continue,
            };
            // Use email as identifier, or token hash if email is empty (e.g., Claude before API call)
            let identifier = if cred.email.is_empty() {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                cred.access_token.hash(&mut hasher);
                format!("token:{:x}", hasher.finish())
            } else {
                cred.email
            };
            let key = (cred.provider, identifier);
            self.daemon_credentials.insert(key, provider_creds);
        }
    }

    /// Get all stored credentials for fetching usage.
    #[cfg_attr(not(feature = "host-gui"), allow(dead_code))]
    pub fn get_credentials(&self) -> Vec<(String, ProviderCredentials)> {
        self.daemon_credentials
            .iter()
            .map(|((provider, _email), creds)| (provider.clone(), creds.clone()))
            .collect()
    }

    /// Register a new container connection.
    pub fn add_container(
        &mut self,
        container_id: String,
        container_name: String,
        working_dir: PathBuf,
        git_sha: String,
        build_timestamp: u64,
    ) {
        self.containers.insert(
            container_id,
            ConnectedContainer::new(container_name, working_dir, git_sha, build_timestamp),
        );
        self.invalidate_cache();
    }

    /// Remove a container (on disconnect).
    pub fn remove_container(&mut self, container_id: &str) {
        self.containers.remove(container_id);
        self.invalidate_cache();
    }

    /// Sync all sessions for a container.
    pub fn sync_sessions(&mut self, container_id: &str, sessions: Vec<SessionInfo>) {
        if let Some(container) = self.containers.get_mut(container_id) {
            eprintln!(
                "[host-state] sync_sessions: {} sessions for container '{}'",
                sessions.len(),
                container_id
            );
            container.sessions.clear();
            for session in &sessions {
                eprintln!(
                    "[host-state]   - {} (feature: {})",
                    session.session_id, session.feature_name
                );
            }
            for session in sessions {
                container
                    .sessions
                    .insert(session.session_id.clone(), session);
            }
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        } else {
            eprintln!(
                "[host-state] WARNING: sync_sessions called for unknown container '{}'",
                container_id
            );
        }
    }

    /// Update a single session.
    pub fn update_session(&mut self, container_id: &str, session: SessionInfo) {
        if let Some(container) = self.containers.get_mut(container_id) {
            eprintln!(
                "[host-state] update_session: {} (feature: {}) in container '{}'",
                session.session_id, session.feature_name, container_id
            );
            container
                .sessions
                .insert(session.session_id.clone(), session);
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        } else {
            eprintln!(
                "[host-state] WARNING: update_session called for unknown container '{}'",
                container_id
            );
        }
    }

    /// Remove a session.
    pub fn remove_session(&mut self, container_id: &str, session_id: &str) {
        if let Some(container) = self.containers.get_mut(container_id) {
            container.sessions.remove(session_id);
            container.last_message_at = Instant::now();
            self.last_update = Instant::now();
            self.invalidate_cache();
        }
    }

    /// Record heartbeat from container.
    pub fn heartbeat(&mut self, container_id: &str) {
        if let Some(container) = self.containers.get_mut(container_id) {
            container.last_message_at = Instant::now();
        }
    }

    /// Get flattened session list for display.
    pub fn sessions(&mut self) -> &[DisplaySession] {
        if self.cached_sessions.is_none() {
            let mut sessions = Vec::new();
            for container in self.containers.values() {
                for session in container.sessions.values() {
                    sessions.push(DisplaySession::new(
                        container.container_name.clone(),
                        session.clone(),
                    ));
                }
            }
            // Sort: AwaitingApproval first, then by updated_at descending
            sessions.sort_by(|a, b| {
                let status_order = |s: &str| match s.to_lowercase().as_str() {
                    "awaitingapproval" | "awaiting_approval" => 0,
                    "running" | "planning" | "reviewing" | "revising" => 1,
                    "error" => 2,
                    "stopped" => 3,
                    "complete" => 4,
                    _ => 5,
                };
                let a_order = status_order(&a.session.status);
                let b_order = status_order(&b.session.status);
                match a_order.cmp(&b_order) {
                    std::cmp::Ordering::Equal => b.session.updated_at.cmp(&a.session.updated_at),
                    other => other,
                }
            });
            self.cached_sessions = Some(sessions);
        }
        self.cached_sessions.as_ref().unwrap()
    }

    /// Count of sessions awaiting approval.
    pub fn approval_count(&self) -> usize {
        self.containers
            .values()
            .flat_map(|c| c.sessions.values())
            .filter(|s| s.status.to_lowercase().contains("approval"))
            .count()
    }

    /// Count of active (non-complete) sessions.
    pub fn active_count(&self) -> usize {
        self.containers
            .values()
            .flat_map(|c| c.sessions.values())
            .filter(|s| s.status.to_lowercase() != "complete")
            .count()
    }

    /// Get account usage info for RPC response.
    pub fn get_account_usage(&self) -> Vec<AccountUsageInfo> {
        self.usage_store
            .get_all_accounts()
            .iter()
            .filter_map(|record| {
                let usage = record.current_usage.as_ref()?;
                Some(AccountUsageInfo {
                    account_id: record.account_id.to_string(),
                    provider: record.provider.clone(),
                    email: record.email.clone(),
                    plan_type: record.plan_type.clone(),
                    rate_limit_tier: usage.rate_limit_tier.clone(),
                    session_percent: usage.session_window.used_percent,
                    session_reset_at: usage.session_window.reset_at.map(|r| r.epoch_seconds),
                    weekly_percent: usage.weekly_window.used_percent,
                    weekly_reset_at: usage.weekly_window.reset_at.map(|r| r.epoch_seconds),
                    fetched_at: usage.fetched_at.clone(),
                    token_valid: usage.token_valid,
                    error: usage.error.clone(),
                })
            })
            .collect()
    }

    fn invalidate_cache(&mut self) {
        self.cached_sessions = None;
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
