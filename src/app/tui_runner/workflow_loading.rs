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

impl ResumableSession {
    pub fn new(feature_name: String, session_id: String, working_dir: PathBuf) -> Self {
        Self {
            feature_name,
            session_id,
            working_dir,
        }
    }
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

/// Load workflow config from a snapshot's stored workflow name.
///
/// This ensures the resumed session uses the same workflow that was originally used,
/// regardless of any changes to the current workflow selection.
///
/// Fallback priority if workflow_name is empty or loading fails:
/// 1. Persisted workflow selection for the snapshot's working directory
/// 2. `./workflow.yaml` in working directory
/// 3. Fallback to claude_only_config()
pub fn load_workflow_from_snapshot(
    snapshot: &crate::session_daemon::SessionSnapshot,
) -> WorkflowConfig {
    // Try to load from stored workflow name first
    if !snapshot.workflow_name.is_empty() {
        match crate::app::load_workflow_by_name(&snapshot.workflow_name) {
            Ok(config) => return config,
            Err(e) => {
                eprintln!(
                    "[planning-agent] Warning: Failed to load workflow '{}' from snapshot: {}. Falling back to current selection.",
                    snapshot.workflow_name, e
                );
            }
        }
    }

    // Fall back to current selection for the snapshot's working directory
    load_workflow_from_selection(&snapshot.working_dir)
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
    if let Ok(selection) = crate::app::WorkflowSelection::load(working_dir) {
        match crate::app::load_workflow_by_name(&selection.workflow) {
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

/// Load workflow config for CLI resume, respecting CLI overrides then snapshot's stored workflow.
///
/// Priority order:
/// 1. `--claude` flag → claude_only_config()
/// 2. `--config <path>` → load from specified file
/// 3. Snapshot's stored workflow_name (preserves original workflow)
/// 4. Persisted workflow selection (fallback for old snapshots)
/// 5. `./workflow.yaml` in working directory
/// 6. Fallback to claude_only_config()
pub fn load_workflow_config_for_resume(
    cli: &Cli,
    snapshot: &crate::session_daemon::SessionSnapshot,
    start: std::time::Instant,
) -> WorkflowConfig {
    // --claude flag takes priority over any config file or snapshot workflow
    if cli.claude {
        debug_log(start, "Using Claude-only workflow config (--claude)");
        let mut cfg = WorkflowConfig::claude_only_config();
        cfg.name = "claude-only".to_string();
        return cfg;
    }

    // --config flag takes priority over snapshot workflow
    if let Some(config_path) = &cli.config {
        let full_path = if config_path.is_absolute() {
            config_path.clone()
        } else {
            snapshot.working_dir.join(config_path)
        };
        match WorkflowConfig::load(&full_path) {
            Ok(mut cfg) => {
                debug_log(start, &format!("Loaded config from {:?}", full_path));
                // Set name from filename if not already set
                if cfg.name.is_empty() {
                    cfg.name = full_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "custom".to_string());
                }
                return cfg;
            }
            Err(e) => {
                eprintln!("[planning-agent] Warning: Failed to load config: {}", e);
            }
        }
    }

    // Use snapshot's stored workflow name (preserves original workflow across resume)
    let cfg = load_workflow_from_snapshot(snapshot);
    debug_log(
        start,
        &format!(
            "Loaded workflow '{}' from snapshot for {}",
            cfg.name,
            snapshot.working_dir.display()
        ),
    );
    cfg
}
