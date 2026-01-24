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

/// Load workflow config from persisted selection or working directory.
///
/// This function handles the non-CLI loading priority:
/// 1. Persisted workflow selection → `~/.planning-agent/state/<wd-hash>/workflow-selection.json`
/// 2. `./workflow.yaml` in working directory → auto-discover
/// 3. Fallback → claude_only_config()
///
/// Use this for dynamic workflow reloading where CLI flags should not apply
/// (e.g., after user explicitly selects a workflow via `/workflow`).
pub fn load_workflow_from_selection(working_dir: &Path) -> WorkflowConfig {
    // Check for persisted workflow selection (per-working-directory)
    if let Ok(selection) = crate::workflow_selection::WorkflowSelection::load(working_dir) {
        match crate::workflow_selection::load_workflow_by_name(&selection.workflow) {
            Ok(cfg) => {
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

    WorkflowConfig::claude_only_config()
}

/// Get workflow config for a session, using context if available or loading from selection.
///
/// This is the primary way to get workflow config during TUI operation:
/// - If session has context, use context.workflow_config (preserves session's original config)
/// - Otherwise, load from current persisted selection
pub fn get_workflow_config_for_session(
    session: &crate::tui::Session,
    working_dir: &Path,
) -> WorkflowConfig {
    session
        .context
        .as_ref()
        .map(|ctx| ctx.workflow_config.clone())
        .unwrap_or_else(|| load_workflow_from_selection(working_dir))
}

/// Load workflow config based on CLI flags and working directory.
///
/// Priority order:
/// 1. `--claude` flag → claude_only_config()
/// 2. `--config <path>` → load from specified file
/// 3. Persisted workflow selection
/// 4. `./workflow.yaml` in working directory
/// 5. Fallback to claude_only_config()
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

    // Delegate to the shared helper for selection/local/fallback logic
    let cfg = load_workflow_from_selection(working_dir);
    debug_log(
        start,
        &format!("Loaded workflow config for {}", working_dir.display()),
    );
    cfg
}
