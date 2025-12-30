use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "planning")]
#[command(about = "Iterative planning workflow orchestrator using Claude Code")]
#[command(version)]
pub struct Cli {
    #[arg(trailing_var_arg = true)]
    pub objective: Vec<String>,

    #[arg(short, long, default_value = "3")]
    pub max_iterations: u32,

    #[arg(short, long)]
    pub continue_workflow: bool,

    #[arg(short, long)]
    pub name: Option<String>,

    #[arg(long)]
    pub working_dir: Option<PathBuf>,

    #[arg(long)]
    pub headless: bool,

    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Resume a stopped session by its ID
    #[arg(long)]
    pub resume_session: Option<String>,

    /// List all available session snapshots
    #[arg(long)]
    pub list_sessions: bool,

    /// Clean up stale session snapshots
    #[arg(long)]
    pub cleanup_sessions: bool,

    /// Days threshold for cleanup (used with --cleanup-sessions)
    #[arg(long)]
    pub older_than: Option<u32>,

    /// Verify implementation against an approved plan.
    /// Accepts either a plan folder path, plan.md file path, or a plan name pattern.
    /// Use "latest" to verify against the most recent plan.
    #[arg(long, value_name = "PLAN_PATH_OR_NAME")]
    pub verify: Option<String>,

    /// List all available plans
    #[arg(long)]
    pub list_plans: bool,
}
