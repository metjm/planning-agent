//! Workflow browser input handling.
//!
//! This module handles keyboard input for the workflow browser overlay,
//! including navigation and selection.

use crate::tui::{Event, TabManager};
use crate::workflow_selection::{load_workflow_by_name, WorkflowSelection};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use tokio::sync::mpsc;

/// Handle input when the workflow browser overlay is open.
pub async fn handle_workflow_browser_input(
    key: crossterm::event::KeyEvent,
    tab_manager: &mut TabManager,
    _output_tx: &mpsc::UnboundedSender<Event>,
) -> Result<bool> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            tab_manager.workflow_browser.select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            tab_manager.workflow_browser.select_prev();
        }
        KeyCode::Enter => {
            if let Some(entry) = tab_manager.workflow_browser.selected_entry() {
                let name = entry.name.clone();
                let working_dir = tab_manager.workflow_browser.working_dir.clone();

                // Save selection
                let selection = WorkflowSelection {
                    workflow: name.clone(),
                };
                if let Err(e) = selection.save(&working_dir) {
                    tab_manager.command_error = Some(format!("Failed to save: {}", e));
                } else {
                    // Update active session's workflow config
                    let session = tab_manager.active_mut();
                    if let Some(ref mut ctx) = session.context {
                        if let Ok(config) = load_workflow_by_name(&name) {
                            ctx.workflow_config = config;
                        }
                    }
                    tab_manager.command_notice = Some(format!("Workflow set to: {}", name));
                }
                tab_manager.workflow_browser.close();
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            tab_manager.workflow_browser.close();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}
