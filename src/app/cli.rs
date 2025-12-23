
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

    #[arg(long, short = 'c')]
    pub config: Option<PathBuf>,
}
