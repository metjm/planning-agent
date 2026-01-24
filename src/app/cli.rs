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
    pub config: Option<PathBuf>,

    /// Use Claude-only workflow (no Codex or other agents)
    #[arg(long, default_value_t = true)]
    pub claude: bool,

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

    /// Internal: Run as session daemon (used by connect-or-spawn)
    #[arg(long, hide = true)]
    pub session_daemon: bool,

    /// Disable session tracking (useful for debugging)
    #[arg(long)]
    pub no_daemon: bool,

    /// Enable git worktree creation (creates isolated branch for planning)
    #[arg(long)]
    pub worktree: bool,

    /// Custom directory for git worktree (for CI/CD scenarios)
    #[arg(long)]
    pub worktree_dir: Option<PathBuf>,

    /// Custom branch name for git worktree (default: planning-agent/<feature>-<session-short>)
    #[arg(long)]
    pub worktree_branch: Option<String>,

    /// Run as host application aggregating sessions from containers
    #[arg(long)]
    pub host: bool,

    /// Port for host mode TCP server (default: 17717)
    #[arg(long, default_value = "17717")]
    pub port: u16,
}
