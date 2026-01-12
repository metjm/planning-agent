mod agents;
mod app;
mod claude_usage;
mod cli_usage;
mod codex_usage;
mod config;
mod diagnostics;
mod gemini_usage;
mod mcp;
mod phases;
mod planning_paths;
pub mod prompt_format;
mod session_store;
mod skills;
mod state;
mod tui;
mod update;
mod verification_state;

use anyhow::Result;
use app::{cli::Cli, headless::run_headless, tui_runner::run_tui, verify::run_headless_verification};
use clap::Parser;
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = skills::install_skills_if_needed() {
        eprintln!("[planning-agent] Warning: Failed to install skills: {}", e);
    }

    let start = std::time::Instant::now();

    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/planning-debug.log")
        {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "\n=== NEW RUN {} ===", now);
        }
    }
    app::util::debug_log(start, "main starting");

    let cli = Cli::parse();
    app::util::debug_log(start, "cli parsed");

    // Handle internal MCP server mode
    if cli.internal_mcp_server {
        return run_mcp_server(&cli);
    }

    // Handle session management commands first (no TUI needed)
    let working_dir = cli
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if cli.list_sessions {
        return list_sessions(&working_dir);
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
        return run_headless_verification(
            plan_path,
            working_dir,
            cli.config.clone(),
        )
        .await;
    }

    // Resume session or normal workflow
    if cli.headless {
        run_headless(cli).await
    } else {
        run_tui(cli, start).await
    }
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
            .ok_or_else(|| anyhow::anyhow!("No plans found. Create a plan first with 'planning <objective>'"));
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
    println!(
        "{:<20} {:<40} Folder",
        "Created", "Feature Name"
    );
    println!("{}", "-".repeat(100));

    for plan in plans {
        // Format timestamp (YYYYMMDD-HHMMSS -> YYYY-MM-DD HH:MM:SS)
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

/// Lists available session snapshots
fn list_sessions(working_dir: &Path) -> Result<()> {
    let snapshots = session_store::list_snapshots(working_dir)?;

    if snapshots.is_empty() {
        println!("No session snapshots found.");
        println!("Snapshots are created when you stop a running workflow.");
        return Ok(());
    }

    println!("Available session snapshots:\n");
    println!(
        "{:<40} {:<20} {:<12} {:<8} Saved At",
        "Session ID", "Feature", "Phase", "Iter"
    );
    println!("{}", "-".repeat(100));

    for snapshot in snapshots {
        println!(
            "{:<40} {:<20} {:<12} {:<8} {}",
            snapshot.workflow_session_id,
            truncate_string(&snapshot.feature_name, 18),
            snapshot.phase,
            snapshot.iteration,
            &snapshot.saved_at[..19].replace('T', " "), // Format timestamp
        );
    }

    println!("\nTo resume a session: planning --resume-session <session-id>");
    Ok(())
}

/// Cleans up old session snapshots
fn cleanup_sessions(working_dir: &Path, older_than: Option<u32>) -> Result<()> {
    let days = older_than.unwrap_or(30);
    let deleted = session_store::cleanup_old_snapshots(working_dir, days)?;

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
        format!("{}...", &s[..max_len - 3])
    }
}

/// Run as an MCP server (internal mode for review feedback collection)
fn run_mcp_server(cli: &Cli) -> Result<()> {
    let plan_content = cli
        .plan_content_b64
        .as_ref()
        .map(|b64| mcp::spawner::decode_plan_content(b64))
        .transpose()?
        .unwrap_or_default();

    let review_prompt = cli
        .review_prompt_b64
        .as_ref()
        .map(|b64| mcp::spawner::decode_review_prompt(b64))
        .transpose()?
        .unwrap_or_default();

    // Create a channel for reviews (we don't actually use the receiver in server mode,
    // but the server needs it for its API)
    let (review_tx, _review_rx) = tokio::sync::mpsc::channel(1);

    let server = mcp::McpReviewServer::new(review_tx, plan_content, review_prompt);
    server.run_sync()
}
