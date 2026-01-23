//! Workflow configuration loading and terminal utilities.
//!
//! Handles loading workflow configs from various sources with proper priority order.

use crate::app::cli::Cli;
use crate::app::util::debug_log;
use crate::config::WorkflowConfig;
use anyhow::Result;
use crossterm::event::PopKeyboardEnhancementFlags;
use std::path::{Path, PathBuf};

/// Information about a session that was successfully stopped and can be resumed.
#[derive(Clone)]
pub struct ResumableSession {
    pub feature_name: String,
    pub session_id: String,
    pub working_dir: PathBuf,
}

pub fn restore_terminal(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        PopKeyboardEnhancementFlags,
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Load workflow config based on CLI flags and working directory.
///
/// Priority order:
/// 1. `--claude` flag → claude_only_config()
/// 2. `--config <path>` → load from specified file
/// 3. Persisted workflow selection → `~/.planning-agent/state/<wd-hash>/workflow-selection.json`
/// 4. `./workflow.yaml` in working directory → auto-discover
/// 5. Fallback → claude_only_config()
pub fn load_workflow_config(
    cli: &Cli,
    working_dir: &Path,
    start: std::time::Instant,
) -> WorkflowConfig {
    // --claude flag takes priority over any config file or selection
    if cli.claude {
        debug_log(start, "Using Claude-only workflow config (--claude)");
        return WorkflowConfig::claude_only_config();
    }

    // --config flag takes priority over saved selection
    if let Some(config_path) = &cli.config {
        let full_path = if config_path.is_absolute() {
            config_path.clone()
        } else {
            working_dir.join(config_path)
        };
        match WorkflowConfig::load(&full_path) {
            Ok(cfg) => {
                debug_log(start, &format!("Loaded config from {:?}", full_path));
                return cfg;
            }
            Err(e) => {
                eprintln!("[planning-agent] Warning: Failed to load config: {}", e);
            }
        }
    }

    // Check for persisted workflow selection (per-working-directory)
    if let Ok(selection) = crate::workflow_selection::WorkflowSelection::load(working_dir) {
        match crate::workflow_selection::load_workflow_by_name(&selection.workflow) {
            Ok(cfg) => {
                debug_log(
                    start,
                    &format!(
                        "Using selected workflow '{}' for {}",
                        selection.workflow,
                        working_dir.display()
                    ),
                );
                return cfg;
            }
            Err(e) => {
                eprintln!(
                    "[planning-agent] Warning: Failed to load selected workflow '{}': {}",
                    selection.workflow, e
                );
            }
        }
    }

    // ./workflow.yaml in working directory
    let default_config_path = working_dir.join("workflow.yaml");
    if default_config_path.exists() {
        match WorkflowConfig::load(&default_config_path) {
            Ok(cfg) => {
                debug_log(start, "Loaded default workflow.yaml");
                return cfg;
            }
            Err(e) => {
                eprintln!(
                    "[planning-agent] Warning: Failed to load workflow.yaml: {}",
                    e
                );
            }
        }
    }

    debug_log(start, "Using built-in claude-only workflow config");
    WorkflowConfig::claude_only_config()
}
