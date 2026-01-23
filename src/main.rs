mod agents;
mod app;
mod change_fingerprint;
mod claude_usage;
mod cli_usage;
mod codex_usage;
mod config;
mod daemon_log;
mod diagnostics;
mod gemini_usage;
mod git_worktree;
mod host;
mod host_protocol;
mod phases;
mod planning_paths;
pub mod prompt_format;
mod rpc;
mod session_daemon;
mod session_logger;
mod session_store;
mod session_tracking;
mod skills;
mod state;
pub mod state_machine;
pub mod structured_logger;
mod tui;
mod update;
mod usage_reset;
mod verification_state;
mod workflow_selection;

use anyhow::Result;
use app::{cli::Cli, tui_runner::run_tui, verify::run_headless_verification};
use clap::Parser;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    if let Err(e) = skills::install_skills_if_needed() {
        eprintln!("[planning-agent] Warning: Failed to install skills: {}", e);
    }

    // Build runtime with fast shutdown - don't wait for blocking tasks
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let result = runtime.block_on(async_main());

    // Shutdown with 100ms timeout - don't wait for slow blocking tasks
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));

    result
}

async fn async_main() -> Result<()> {
    let start = std::time::Instant::now();

    // Log startup message to session-scoped startup log (merged into session log later)
    {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        session_logger::log_startup(&format!("=== NEW RUN {} ===", now));
    }
    session_logger::log_startup("main starting");

    let cli = Cli::parse();
    session_logger::log_startup("cli parsed");

    // Handle session daemon mode (internal, used by connect-or-spawn)
    if cli.session_daemon {
        return session_daemon::run_daemon_rpc().await;
    }

    // Handle host mode (desktop GUI aggregating container sessions)
    if cli.host {
        return run_host(cli.port).await;
    }

    // Handle session management commands first (no TUI needed)
    let working_dir = cli
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if cli.list_sessions {
        return list_sessions(&working_dir).await;
    }

    if cli.cleanup_sessions {
        return cleanup_sessions(&working_dir, cli.older_than);
    }

    // Handle list-plans command
    if cli.list_plans {
        return list_plans();
    }

    // Handle verification mode
    if let Some(ref plan_spec) = cli.verify {
        let plan_path = resolve_plan_path(plan_spec)?;
        return run_headless_verification(plan_path, working_dir, cli.config.clone()).await;
    }

    // Run TUI workflow
    let result = run_tui(cli, start).await;
    session_logger::log_startup("main function returning");
    result
}

/// Resolves a plan specification (path, name pattern, or "latest") to a full path.
fn resolve_plan_path(spec: &str) -> Result<PathBuf> {
    // Check if it's already a valid path
    let path = PathBuf::from(spec);
    if path.exists() {
        return Ok(path);
    }

    // Handle "latest" keyword
    if spec.eq_ignore_ascii_case("latest") {
        return planning_paths::latest_plan()?
            .map(|p| p.path)
            .ok_or_else(|| {
                anyhow::anyhow!("No plans found. Create a plan first with 'planning <objective>'")
            });
    }

    // Try to find by pattern
    planning_paths::find_plan(spec)?
        .map(|p| p.path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No plan found matching '{}'. Use --list-plans to see available plans.",
                spec
            )
        })
}

/// Lists all available plans
fn list_plans() -> Result<()> {
    let plans = planning_paths::list_plans()?;

    if plans.is_empty() {
        println!("No plans found.");
        println!("Create a plan with: planning <objective>");
        return Ok(());
    }

    println!("Available plans:\n");
    println!("{:<20} {:<40} Folder", "Created", "Feature Name");
    println!("{}", "-".repeat(100));

    for plan in plans {
        // Format timestamp (YYYYMMDD-HHMMSS -> YYYY-MM-DD HH:MM:SS)
        // Timestamp format is ASCII digits + dash, so byte indexing is safe
        #[allow(clippy::string_slice)]
        let formatted_ts = if plan.timestamp.len() >= 15 {
            format!(
                "{}-{}-{} {}:{}:{}",
                &plan.timestamp[0..4],
                &plan.timestamp[4..6],
                &plan.timestamp[6..8],
                &plan.timestamp[9..11],
                &plan.timestamp[11..13],
                &plan.timestamp[13..15],
            )
        } else {
            plan.timestamp.clone()
        };

        println!(
            "{:<20} {:<40} {}",
            formatted_ts,
            truncate_string(&plan.feature_name, 38),
            plan.folder_name
        );
    }

    println!("\nTo verify a plan: planning --verify <plan-name-or-path>");
    println!("To verify latest plan: planning --verify latest");
    Ok(())
}

/// A unified session entry for display (merges live daemon data with disk snapshots)
struct SessionDisplayEntry {
    session_id: String,
    feature_name: String,
    phase: String,
    iteration: u32,
    workflow_status: String,
    liveness: String,
    last_seen: String,
    last_seen_at: String, // Raw timestamp for sorting
    is_live: bool,
}

/// Formats a timestamp as relative time (e.g., "2m ago", "1h ago")
fn format_relative_time(timestamp: &str) -> String {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).or_else(|_| {
        // Try parsing without timezone (some timestamps are ISO format without tz)
        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S")
            .or_else(|e| {
                // Truncate to first 19 chars if longer (e.g., "2024-01-15T10:30:00.123Z" -> "2024-01-15T10:30:00")
                if let Some(truncated) = timestamp.get(..19) {
                    chrono::NaiveDateTime::parse_from_str(truncated, "%Y-%m-%dT%H:%M:%S")
                } else {
                    Err(e)
                }
            })
            .map(|dt| dt.and_utc().fixed_offset())
    });

    match parsed {
        Ok(dt) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt.with_timezone(&chrono::Utc));

            if duration.num_seconds() < 60 {
                "just now".to_string()
            } else if duration.num_minutes() < 60 {
                format!("{}m ago", duration.num_minutes())
            } else if duration.num_hours() < 24 {
                format!("{}h ago", duration.num_hours())
            } else {
                format!("{}d ago", duration.num_days())
            }
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Lists available sessions (live from daemon + disk snapshots)
async fn list_sessions(_working_dir: &Path) -> Result<()> {
    let mut entries: Vec<SessionDisplayEntry> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Try to get live sessions from daemon using new RPC client
    let daemon_client = session_daemon::RpcClient::new(false).await;
    let daemon_connected = daemon_client.is_connected();

    if daemon_connected {
        if let Ok(live_sessions) = daemon_client.list().await {
            for record in live_sessions {
                seen_ids.insert(record.workflow_session_id.clone());
                entries.push(SessionDisplayEntry {
                    session_id: record.workflow_session_id,
                    feature_name: record.feature_name,
                    phase: record.phase,
                    iteration: record.iteration,
                    workflow_status: record.workflow_status,
                    liveness: format!("{}", record.liveness),
                    last_seen: format_relative_time(&record.last_heartbeat_at),
                    last_seen_at: record.last_heartbeat_at,
                    is_live: record.liveness == session_daemon::LivenessState::Running,
                });
            }
        }
    }

    // Load disk snapshots and merge (add ones not already in live list)
    if let Ok(snapshots) = session_store::list_snapshots() {
        for snapshot in snapshots {
            if !seen_ids.contains(&snapshot.workflow_session_id) {
                seen_ids.insert(snapshot.workflow_session_id.clone());
                entries.push(SessionDisplayEntry {
                    session_id: snapshot.workflow_session_id,
                    feature_name: snapshot.feature_name,
                    phase: snapshot.phase,
                    iteration: snapshot.iteration,
                    workflow_status: "Stopped".to_string(),
                    liveness: "Stopped".to_string(),
                    last_seen: format_relative_time(&snapshot.saved_at),
                    last_seen_at: snapshot.saved_at,
                    is_live: false,
                });
            }
        }
    }

    // Show daemon connection status
    if daemon_connected {
        println!("Daemon: Connected");
    } else {
        println!("Daemon: Offline (showing snapshots only)");
    }

    if entries.is_empty() {
        println!("\nNo sessions found.");
        println!("Sessions are created when you start a workflow.");
        return Ok(());
    }

    // Sort: live/running first, then by last_seen_at (most recent first)
    entries.sort_by(|a, b| {
        match (a.is_live, b.is_live) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.last_seen_at.cmp(&a.last_seen_at), // Secondary sort by timestamp (descending)
        }
    });

    println!("\nSessions:\n");
    println!(
        "{:<36} {:<16} {:<12} {:<4} {:<10} {:<12} Last Seen",
        "Session ID", "Feature", "Phase", "Iter", "Status", "Liveness"
    );
    println!("{}", "-".repeat(105));

    for entry in entries {
        println!(
            "{:<36} {:<16} {:<12} {:<4} {:<10} {:<12} {}",
            truncate_string(&entry.session_id, 34),
            truncate_string(&entry.feature_name, 14),
            truncate_string(&entry.phase, 10),
            entry.iteration,
            truncate_string(&entry.workflow_status, 8),
            entry.liveness,
            entry.last_seen,
        );
    }

    println!("\nTo resume a session: planning --resume-session <session-id>");
    println!("Note: Use /sessions in the TUI for an interactive browser.");
    Ok(())
}

/// Cleans up old session snapshots
fn cleanup_sessions(_working_dir: &Path, older_than: Option<u32>) -> Result<()> {
    let days = older_than.unwrap_or(30);
    let deleted = session_store::cleanup_old_snapshots(days)?;

    if deleted.is_empty() {
        println!("No sessions older than {} days found.", days);
    } else {
        println!("Cleaned up {} session snapshot(s):", deleted.len());
        for id in &deleted {
            println!("  - {}", id);
        }
    }

    Ok(())
}

/// Truncates a string to a max length, adding "..." if truncated
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let prefix = s.get(..max_len.saturating_sub(3)).unwrap_or("");
        format!("{}...", prefix)
    }
}

/// Run the host application with GUI and TCP server.
#[cfg(feature = "host-gui")]
async fn run_host(port: u16) -> Result<()> {
    use crate::host::gui::app::HostApp;
    use crate::host::server::run_server;
    use crate::host::state::HostState;
    use eframe::egui;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex};

    let state = Arc::new(Mutex::new(HostState::new()));
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Spawn TCP server in background
    let server_state = state.clone();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = run_server(port, server_state, event_tx).await {
            eprintln!("[host] Server error: {}", e);
        }
    });

    // Run GUI on main thread (required for macOS)
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 600.0])
            .with_min_inner_size([800.0, 400.0])
            .with_title("Planning Agent Host"),
        ..Default::default()
    };

    // This blocks until window is closed
    let gui_result = eframe::run_native(
        "Planning Agent Host",
        native_options,
        Box::new(move |_cc| Ok(Box::new(HostApp::new(state, event_rx, port)))),
    );

    // Cleanup
    server_handle.abort();

    gui_result.map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}

/// Stub for host mode when GUI feature is not enabled.
#[cfg(not(feature = "host-gui"))]
async fn run_host(_port: u16) -> Result<()> {
    anyhow::bail!(
        "Host mode requires the 'host-gui' feature.\n\
         Build with: cargo build --features host-gui\n\
         \n\
         Note: This feature requires GUI system libraries:\n\
         - macOS: Available by default\n\
         - Linux: Install libgtk-3-dev, libatk1.0-dev, libpango1.0-dev\n\
         - Windows: Available by default"
    )
}
