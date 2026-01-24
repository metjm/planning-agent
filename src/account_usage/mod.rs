//! Account usage tracking via direct HTTP API calls.
//!
//! This module provides centralized account usage tracking that:
//! - Fetches usage data directly via HTTP APIs (not CLI invocation)
//! - Uses the host as the primary usage fetcher, with daemon as fallback
//! - Persists historical usage data in the host
//! - Tracks all accounts by email as the primary identifier

pub mod api_client;
pub mod credentials;
pub mod fetcher;
pub mod store;
pub mod types;
