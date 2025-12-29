mod agents;
mod app;
mod claude_usage;
mod cli_usage;
mod codex_usage;
mod config;
mod gemini_usage;
mod phases;
mod planning_dir;
mod session_store;
mod skills;
mod state;
mod tui;
mod update;

use anyhow::Result;
use app::{cli::Cli, headless::run_headless, tui_runner::run_tui};
use clap::Parser;
use std::path::PathBuf;

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

    // Resume session or normal workflow
    if cli.headless {
        run_headless(cli).await
    } else {
        run_tui(cli, start).await
    }
}

/// Lists available session snapshots
fn list_sessions(working_dir: &PathBuf) -> Result<()> {
    let snapshots = session_store::list_snapshots(working_dir)?;

    if snapshots.is_empty() {
        println!("No session snapshots found.");
        println!("Snapshots are created when you stop a running workflow.");
        return Ok(());
    }

    println!("Available session snapshots:\n");
    println!(
        "{:<40} {:<20} {:<12} {:<8} {}",
        "Session ID", "Feature", "Phase", "Iter", "Saved At"
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
fn cleanup_sessions(working_dir: &PathBuf, older_than: Option<u32>) -> Result<()> {
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
