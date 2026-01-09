
mod approval_input;
mod events;
mod implementation_input;
mod input;

use crate::app::cli::Cli;
use crate::app::headless::extract_feature_name;
use crate::app::util::{debug_log, format_window_title};
use crate::app::workflow::run_workflow_with_config;
use crate::app::workflow_common::pre_create_plan_files;
use crate::cli_usage;
use crate::config::WorkflowConfig;
use crate::planning_paths;
use crate::state::State;
use crate::tui::{
    Event, EventHandler, InputMode, SessionStatus, TabManager, TerminalTitleManager,
};
use crate::update;
use anyhow::{Context, Result};
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use events::{check_workflow_completions, handle_init_completion, process_event};

pub type InitHandle = Option<(
    usize,
    tokio::task::JoinHandle<Result<(State, PathBuf, String)>>,
)>;

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

pub async fn run_tui(cli: Cli, start: std::time::Instant) -> Result<()> {
    debug_log(start, "run_tui starting");

    crossterm::terminal::enable_raw_mode()?;
    debug_log(start, "raw mode enabled");
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    debug_log(start, "alternate screen entered");

    let keyboard_enhancement_enabled = match crossterm::execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )
    ) {
        Ok(_) => {
            debug_log(start, "keyboard enhancement enabled successfully");
            true
        }
        Err(e) => {
            debug_log(start, &format!("keyboard enhancement failed: {}", e));
            false
        }
    };
    let _ = keyboard_enhancement_enabled;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;
    debug_log(start, "terminal created");

    let title_manager = TerminalTitleManager::new();
    title_manager.save_title();
    title_manager.set_title("Planning Agent");
    let mut last_title = "Planning Agent".to_string();
    debug_log(start, "title manager initialized");

    let mut tab_manager = TabManager::new();
    debug_log(start, "tab manager created");
    let mut event_handler = EventHandler::new(Duration::from_millis(100));
    debug_log(start, "event handler created");
    let output_tx = event_handler.sender();

    {
        let usage_tx = event_handler.sender();
        tokio::spawn(async move {
            loop {
                let usage = tokio::task::spawn_blocking(cli_usage::fetch_all_provider_usage_sync)
                    .await
                    .unwrap_or_else(|_| cli_usage::AccountUsage::default());
                let _ = usage_tx.send(Event::AccountUsageUpdate(usage));
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
    }
    debug_log(start, "account usage fetch task spawned");

    if update::BUILD_SHA != "unknown" {
        let update_tx = event_handler.sender();
        tokio::spawn(async move {
            let status = tokio::task::spawn_blocking(update::check_for_update)
                .await
                .unwrap_or_else(|_| {
                    update::UpdateStatus::CheckFailed("Task panicked".to_string())
                });
            let _ = update_tx.send(Event::UpdateStatusReceived(status));
        });
        debug_log(start, "update check task spawned");
    }

    // Spawn background file index task for @-mention auto-complete
    {
        let file_index_tx = event_handler.sender();
        let file_index_working_dir = cli
            .working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));
        tokio::spawn(async move {
            let index = tokio::task::spawn_blocking(move || {
                crate::tui::file_index::build_file_index(&file_index_working_dir)
            })
            .await
            .unwrap_or_else(|_| crate::tui::file_index::FileIndex::with_error());
            let _ = file_index_tx.send(Event::FileIndexReady(index));
        });
        debug_log(start, "file index task spawned");
    }

    let working_dir = cli
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("Failed to get current directory"));

    // Canonicalize working_dir for absolute paths in prompts (matching headless behavior)
    let working_dir = std::fs::canonicalize(&working_dir).unwrap_or(working_dir);

    if update::consume_update_marker(&working_dir) {
        tab_manager.update_notice = Some("Update installed successfully!".to_string());
        debug_log(start, "update-installed marker consumed");
    }

    let workflow_config = load_workflow_config(&cli, &working_dir, start);

    let objective = cli.objective.join(" ").trim().to_string();

    if cli.continue_workflow && cli.name.is_none() {
        restore_terminal(&mut terminal)?;
        anyhow::bail!("--continue requires --name to specify which workflow to continue");
    }

    let mut init_handle: InitHandle = None;

    let first_session_id = tab_manager.active().id;

    // Handle session resume if requested
    if let Some(ref session_id) = cli.resume_session {
        debug_log(start, &format!("resuming session: {}", session_id));

        // Load the snapshot
        let snapshot = match crate::session_store::load_snapshot(&working_dir, session_id) {
            Ok(s) => s,
            Err(e) => {
                restore_terminal(&mut terminal)?;
                anyhow::bail!("Failed to load session '{}': {}", session_id, e);
            }
        };

        // Check for conflict with current state file
        if snapshot.state_path.exists() {
            let current_state = State::load(&snapshot.state_path).ok();
            if let Some(ref cs) = current_state {
                if let Some(conflict_msg) = crate::session_store::check_conflict(&snapshot, cs) {
                    let first_session = tab_manager.active_mut();
                    first_session.add_output(format!("[warning] {}", conflict_msg));
                    first_session.add_output("[warning] Using snapshot state. State file will be overwritten.".to_string());
                }
            }
        }

        // Restore the session from snapshot
        let first_session = tab_manager.active_mut();
        let restored_state = snapshot.workflow_state.clone();
        *first_session = crate::tui::Session::from_ui_state(
            snapshot.ui_state.clone(),
            Some(restored_state.clone()),
        );
        first_session.add_output(format!("[planning] Resumed session: {}", session_id));
        first_session.add_output(format!(
            "[planning] Feature: {}, Phase: {:?}, Iteration: {}",
            restored_state.feature_name, restored_state.phase, restored_state.iteration
        ));

        // Set up for workflow continuation
        first_session.status = SessionStatus::Planning;
        first_session.input_mode = InputMode::Normal;

        // Spawn workflow continuation
        let init_tx = output_tx.clone();
        let state_path = snapshot.state_path.clone();
        let mut state = snapshot.workflow_state;

        // Note: total_elapsed_before_resume_ms can be used for elapsed time tracking
        let _total_elapsed_before = snapshot.total_elapsed_before_resume_ms;

        let handle = tokio::spawn(async move {
            let _ = init_tx.send(Event::Output("[planning] Continuing workflow...".to_string()));

            // Save state to ensure state file is in sync with snapshot
            state.set_updated_at();
            state.save(&state_path)?;

            let _ = init_tx.send(Event::StateUpdate(state.clone()));

            let feature_name = state.feature_name.clone();
            Ok::<_, anyhow::Error>((state, state_path, feature_name))
        });
        debug_log(start, "resume init task spawned");

        init_handle = Some((first_session_id, handle));

        // Store the elapsed time from before resume for cost tracking
        if let Some(session) = tab_manager.sessions.iter_mut().find(|s| s.id == first_session_id) {
            session.total_cost = snapshot.ui_state.total_cost;
        }
    } else if objective.is_empty() {

        let first_session = tab_manager.active_mut();
        first_session.input_mode = InputMode::NamingTab;
        first_session.status = SessionStatus::InputPending;
        debug_log(start, "interactive mode - waiting for user input");
    } else {

        let first_session = tab_manager.active_mut();
        first_session.input_mode = InputMode::Normal;
        first_session.status = SessionStatus::Planning;

        let init_tx = output_tx.clone();
        let init_working_dir = working_dir.clone();
        let init_objective = objective.clone();
        let init_name = cli.name.clone();
        let init_continue = cli.continue_workflow;
        let init_max_iterations = cli.max_iterations;

        let handle = tokio::spawn(async move {
            let _ = init_tx.send(Event::Output("[planning] Initializing...".to_string()));

            let feature_name = if let Some(name) = init_name {
                name
            } else {
                extract_feature_name(&init_objective, Some(&init_tx)).await?
            };

            let state_path = planning_paths::state_path(&init_working_dir, &feature_name)?;

            let mut state = if init_continue {
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Loading existing workflow: {}",
                    feature_name
                )));
                State::load(&state_path)?
            } else {
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Starting new workflow: {}",
                    feature_name
                )));
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Objective: {}",
                    init_objective
                )));
                State::new(&feature_name, &init_objective, init_max_iterations)?
            };

            // Pre-create plan folder and files (in ~/.planning-agent/plans/)
            pre_create_plan_files(&state).context("Failed to pre-create plan files")?;

            state.set_updated_at();
            state.save(&state_path)?;

            let _ = init_tx.send(Event::StateUpdate(state.clone()));

            Ok::<_, anyhow::Error>((state, state_path, feature_name))
        });
        debug_log(start, "init task spawned");

        init_handle = Some((first_session_id, handle));
    }
    let mut should_quit = false;

    debug_log(start, "entering main loop");

    const MAX_EVENTS_PER_FRAME: usize = 50;

    loop {

        terminal.draw(|frame| crate::tui::ui::draw(frame, &tab_manager))?;

        let first_event = event_handler.next().await?;
        let mut events_to_process = vec![first_event];

        while events_to_process.len() < MAX_EVENTS_PER_FRAME {
            match event_handler.try_next() {
                Some(event) => events_to_process.push(event),
                None => break,
            }
        }

        for event in events_to_process {
            let quit_requested = process_event(
                event,
                &mut tab_manager,
                &mut terminal,
                &output_tx,
                &working_dir,
                &cli,
                &workflow_config,
                &mut init_handle,
                first_session_id,
            )
            .await?;

            if quit_requested {
                should_quit = true;
            }
        }

        let new_title = format_window_title(&tab_manager);
        if new_title != last_title {
            title_manager.set_title(&new_title);
            last_title = new_title;
        }

        if should_quit {
            break;
        }

        if let Some((session_id, handle)) = init_handle.take() {
            if handle.is_finished() {
                handle_init_completion(
                    session_id,
                    handle,
                    &mut tab_manager,
                    &working_dir,
                    &workflow_config,
                    &output_tx,
                )
                .await;
            } else {
                init_handle = Some((session_id, handle));
            }
        }

        check_workflow_completions(
            &mut tab_manager,
            &working_dir,
            &workflow_config,
            &output_tx,
        )
        .await;
    }

    title_manager.restore_title();
    restore_terminal(&mut terminal)?;

    Ok(())
}

fn load_workflow_config(
    cli: &Cli,
    working_dir: &Path,
    start: std::time::Instant,
) -> WorkflowConfig {
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
                debug_log(start, "Falling back to built-in multi-agent workflow config");
            }
        }
    } else {
        let default_config_path = working_dir.join("workflow.yaml");
        if default_config_path.exists() {
            match WorkflowConfig::load(&default_config_path) {
                Ok(cfg) => {
                    debug_log(start, "Loaded default workflow.yaml");
                    return cfg;
                }
                Err(e) => {
                    eprintln!("[planning-agent] Warning: Failed to load workflow.yaml: {}", e);
                    debug_log(start, "Falling back to built-in multi-agent workflow config");
                }
            }
        } else {
            debug_log(start, "Using built-in multi-agent workflow config");
        }
    }
    WorkflowConfig::default_config()
}
