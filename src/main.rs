mod agents;
mod app;
mod claude_usage;
mod cli_usage;
mod codex_usage;
mod config;
mod gemini_usage;
mod phases;
mod planning_dir;
mod skills;
mod state;
mod tui;
mod update;

use anyhow::Result;
use app::{cli::Cli, headless::run_headless, tui_runner::run_tui};
use clap::Parser;

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

    if cli.headless {
        run_headless(cli).await
    } else {
        run_tui(cli, start).await
    }
}
