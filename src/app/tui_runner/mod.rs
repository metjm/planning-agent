
mod approval_input;
mod events;
mod input;
mod input_naming;
mod session_browser_input;
pub mod slash_commands;
pub mod snapshot_helper;
mod workflow_lifecycle;

use crate::app::cli::Cli;
use crate::app::util::{build_resume_command, debug_log, extract_feature_name, format_window_title};
use crate::app::workflow::run_workflow_with_config;
use crate::app::workflow_common::pre_create_session_folder_with_working_dir;
use crate::cli_usage;
use crate::config::WorkflowConfig;
use crate::planning_paths;
use crate::state::State;
use crate::tui::{
    Event, EventHandler, InputMode, SessionStatus, TabManager, TerminalTitleManager,
    WorkflowCommand,
};
use crate::update;
use anyhow::{Context, Result};
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use events::process_event;
pub use workflow_lifecycle::{check_workflow_completions, handle_init_completion};

pub type InitHandle = Option<(
    usize,
    tokio::task::JoinHandle<Result<(State, PathBuf, String, PathBuf)>>,
)>;


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

    // Set up signal handlers for graceful shutdown
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate()
    ).expect("Failed to create SIGTERM handler");

    #[cfg(unix)]
    let mut sigint = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::interrupt()
    ).expect("Failed to create SIGINT handler");

    debug_log(start, "signal handlers created");

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

    // Spawn background version info fetch task
    {
        let version_tx = event_handler.sender();
        tokio::spawn(async move {
            let version_info = tokio::task::spawn_blocking(update::get_cached_or_fetch_version_info)
                .await
                .unwrap_or(None);
            let _ = version_tx.send(Event::VersionInfoReceived(version_info));
        });
        debug_log(start, "version info task spawned");
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

    // Spawn daemon in background before starting subscription task
    // This ensures daemon is available when subscription tries to connect
    // Uses spawn_blocking since SessionDaemonClient::new() is sync and may take up to 2s
    {
        let daemon_spawn_tx = event_handler.sender();
        tokio::spawn(async move {
            // Run blocking daemon spawn in dedicated thread
            let connected = tokio::task::spawn_blocking(|| {
                let client = crate::session_daemon::client::SessionDaemonClient::new(false);
                client.is_connected()
            })
            .await
            .unwrap_or(false);

            // Send initial status if daemon was already running
            if connected {
                let _ = daemon_spawn_tx.send(Event::DaemonReconnected);
            }
        });
        debug_log(start, "daemon spawn task started");
    }

    // Spawn daemon subscription task for push notifications
    {
        let daemon_tx = event_handler.sender();
        tokio::spawn(async move {
            // Helper to log daemon events
            fn daemon_log(msg: &str) {
                use std::io::Write;
                if let Ok(home) = crate::planning_paths::planning_agent_home_dir() {
                    let log_path = home.join("daemon-debug.log");
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                    {
                        let now = chrono::Local::now().format("%H:%M:%S%.3f");
                        let _ = writeln!(f, "[{}] [tui_runner] {}", now, msg);
                    }
                }
            }

            daemon_log("subscription task started");
            loop {
                daemon_log("attempting to connect...");
                // Try to connect and subscribe
                if let Some(mut subscription) =
                    crate::session_daemon::DaemonSubscription::connect().await
                {
                    daemon_log("connected! sending DaemonReconnected event");
                    let _ = daemon_tx.send(Event::DaemonReconnected);

                    // Forward all push notifications to event system
                    daemon_log("entering recv loop");
                    while let Some(msg) = subscription.recv().await {
                        daemon_log(&format!("received message: {:?}", msg));
                        match msg {
                            crate::session_daemon::protocol::DaemonMessage::SessionChanged(
                                record,
                            ) => {
                                daemon_log("forwarding SessionChanged event");
                                let _ = daemon_tx.send(Event::DaemonSessionChanged(record));
                            }
                            crate::session_daemon::protocol::DaemonMessage::Restarting { .. } => {
                                daemon_log("daemon is restarting, breaking recv loop");
                                break;
                            }
                            _ => {
                                daemon_log("ignoring other message type");
                            }
                        }
                    }
                    daemon_log("recv loop ended, sending DaemonDisconnected event");
                    let _ = daemon_tx.send(Event::DaemonDisconnected);
                } else {
                    daemon_log("connect() returned None");
                }

                daemon_log("waiting 500ms before retry...");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        debug_log(start, "daemon subscription task spawned");
    }

    // Create cancellation token for periodic snapshot task
    let (snapshot_cancel_tx, mut snapshot_cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn periodic snapshot save task for crash recovery
    {
        let snapshot_tx = event_handler.sender();
        tokio::spawn(async move {
            // Initial delay to avoid saving immediately on start
            tokio::time::sleep(Duration::from_secs(30)).await;
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if snapshot_tx.send(Event::SnapshotRequest).is_err() {
                            // Channel closed, exit the task
                            break;
                        }
                    }
                    _ = &mut snapshot_cancel_rx => {
                        // Cancellation requested, exit cleanly
                        break;
                    }
                }
            }
        });
        debug_log(start, "periodic snapshot task spawned");
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
        first_session.name = restored_state.feature_name.clone();
        first_session.add_output(format!("[planning] Resumed session: {}", session_id));
        first_session.add_output(format!(
            "[planning] Feature: {}, Phase: {:?}, Iteration: {}",
            restored_state.feature_name, restored_state.phase, restored_state.iteration
        ));
        first_session.add_output("[planning] Continuing workflow...".to_string());

        // Store the elapsed time from before resume for cost tracking
        first_session.total_cost = snapshot.ui_state.total_cost;

        // Save state to ensure state file is in sync with snapshot
        let state_path = snapshot.state_path.clone();
        let mut state = snapshot.workflow_state;
        state.set_updated_at();
        state.save(&state_path)?;

        let _ = output_tx.send(Event::StateUpdate(state.clone()));

        // Set up session context BEFORE starting the workflow
        // This enables proper working directory tracking for cross-directory resume
        let context = crate::tui::SessionContext::from_snapshot(
            snapshot.working_dir.clone(),
            state_path.clone(),
            state.worktree_info.as_ref(),
            workflow_config.clone(),
        );
        first_session.context = Some(context);

        // Use the shared resume helper (same as /sessions overlay)
        workflow_lifecycle::start_resumed_workflow(
            first_session,
            state,
            state_path,
            &working_dir,
            &workflow_config,
            &output_tx,
        );
        debug_log(start, "resume workflow started via start_resumed_workflow");
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

        // Capture worktree-related CLI flags before tokio::spawn
        let worktree_flag = cli.worktree;
        let custom_worktree_dir = cli.worktree_dir.clone();
        let custom_worktree_branch = cli.worktree_branch.clone();

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

            // Set up git worktree if in a git repository (and not disabled)
            // Check if state already has worktree_info (--continue case)
            let effective_working_dir = if let Some(ref existing_wt) = state.worktree_info {
                // Worktree already exists from previous session (--continue case)
                // Note: Even if --worktree is not passed, we respect the existing worktree
                // because changes already exist there. --worktree only affects NEW sessions.
                // Validate it still exists and is a valid git worktree
                if crate::git_worktree::is_valid_worktree(&existing_wt.worktree_path) {
                    let _ = init_tx.send(Event::Output(format!(
                        "[planning] Reusing existing worktree: {}",
                        existing_wt.worktree_path.display()
                    )));
                    let _ = init_tx.send(Event::Output(format!(
                        "[planning] Branch: {}",
                        existing_wt.branch_name
                    )));
                    existing_wt.worktree_path.clone()
                } else {
                    // Worktree is gone or invalid - clear it and fall back
                    let _ = init_tx.send(Event::Output(
                        "[planning] Warning: Previous worktree no longer valid".to_string()
                    ));
                    let _ = init_tx.send(Event::Output(format!(
                        "[planning] Falling back to: {}",
                        existing_wt.original_dir.display()
                    )));
                    let original = existing_wt.original_dir.clone();
                    state.worktree_info = None;
                    original
                }
            } else if !worktree_flag {
                // Worktree is disabled by default
                init_working_dir.clone()
            } else {
                // No existing worktree, create a new one
                // Get session directory for worktree (graceful fallback if it fails)
                let session_dir = match crate::planning_paths::session_dir(&state.workflow_session_id) {
                    Ok(dir) => dir,
                    Err(e) => {
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Warning: Could not get session directory: {}",
                            e
                        )));
                        let _ = init_tx.send(Event::Output(
                            "[planning] Continuing with original directory".to_string()
                        ));
                        return Ok::<_, anyhow::Error>((state, state_path, feature_name, init_working_dir.clone()));
                    }
                };

                // Use custom worktree dir if provided, otherwise use session_dir
                let worktree_base = custom_worktree_dir
                    .as_ref()
                    .map(|d| d.to_path_buf())
                    .unwrap_or(session_dir);

                match crate::git_worktree::create_session_worktree(
                    &init_working_dir,
                    &state.workflow_session_id,
                    &feature_name,
                    &worktree_base,
                    custom_worktree_branch.as_deref(),
                ) {
                    crate::git_worktree::WorktreeSetupResult::Created(info) => {
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Created git worktree at: {}",
                            info.worktree_path.display()
                        )));
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Working on branch: {}",
                            info.branch_name
                        )));
                        if let Some(ref source) = info.source_branch {
                            let _ = init_tx.send(Event::Output(format!(
                                "[planning] Will merge into: {}",
                                source
                            )));
                        }

                        // Warn about submodules if present
                        if info.has_submodules {
                            let _ = init_tx.send(Event::Output(
                                "[planning] Warning: Repository has submodules".to_string()
                            ));
                            let _ = init_tx.send(Event::Output(
                                "[planning] Submodules may not be initialized in the worktree.".to_string()
                            ));
                            let _ = init_tx.send(Event::Output(
                                "[planning] Run 'git submodule update --init' in the worktree if needed.".to_string()
                            ));
                        }

                        let wt_state = crate::state::WorktreeState {
                            worktree_path: info.worktree_path.clone(),
                            branch_name: info.branch_name,
                            source_branch: info.source_branch,
                            original_dir: info.original_dir,
                        };
                        state.worktree_info = Some(wt_state);
                        info.worktree_path
                    }
                    crate::git_worktree::WorktreeSetupResult::NotAGitRepo => {
                        let _ = init_tx.send(Event::Output(
                            "[planning] Not a git repository, using original directory".to_string()
                        ));
                        init_working_dir.clone()
                    }
                    crate::git_worktree::WorktreeSetupResult::Failed(err) => {
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Warning: Git worktree setup failed: {}",
                            err
                        )));
                        let _ = init_tx.send(Event::Output(
                            "[planning] Continuing with original directory".to_string()
                        ));
                        init_working_dir.clone()
                    }
                }
            };

            // Pre-create plan folder and files using effective_working_dir
            pre_create_session_folder_with_working_dir(&state, Some(&effective_working_dir))
                .context("Failed to pre-create plan files")?;

            state.set_updated_at();
            state.save(&state_path)?;

            let _ = init_tx.send(Event::StateUpdate(state.clone()));

            // Return effective_working_dir along with state, state_path, feature_name
            Ok::<_, anyhow::Error>((state, state_path, feature_name, effective_working_dir))
        });
        debug_log(start, "init task spawned");

        init_handle = Some((first_session_id, handle));
    }
    let mut resumable_sessions: Vec<ResumableSession> = Vec::new();
    let mut quit_requested = false;

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
            if process_event(
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
            .await?
            {
                quit_requested = true;
            }
        }

        // Check for signals
        #[cfg(unix)]
        {
            use std::task::Poll;
            use std::pin::Pin;

            let signal_received = std::future::poll_fn(|cx| {
                if Pin::new(&mut sigterm).poll_recv(cx).is_ready() {
                    return Poll::Ready(true);
                }
                if Pin::new(&mut sigint).poll_recv(cx).is_ready() {
                    return Poll::Ready(true);
                }
                Poll::Ready(false)
            }).await;

            if signal_received {
                debug_log(start, "Signal received");
                quit_requested = true;
            }
        }

        // Handle quit: save state and exit immediately
        if quit_requested {
            debug_log(start, "Quit requested, saving snapshots");
            for session in tab_manager.sessions_mut() {
                // Save snapshot if we have state
                if let Some(ref state) = session.workflow_state {
                    debug_log(start, &format!("Saving snapshot for session {}", state.workflow_session_id));
                    if let Err(e) = snapshot_helper::create_and_save_snapshot(session, state, &working_dir) {
                        debug_log(start, &format!("Failed to save snapshot: {}", e));
                    } else {
                        debug_log(start, "Snapshot saved successfully");
                        resumable_sessions.push(ResumableSession {
                            feature_name: state.feature_name.clone(),
                            session_id: state.workflow_session_id.clone(),
                            working_dir: working_dir.clone(),
                        });
                    }
                }
                // Send stop command and drop channels to unblock workflows
                if session.workflow_handle.is_some() {
                    if let Some(ref tx) = session.workflow_control_tx {
                        let _ = tx.try_send(WorkflowCommand::Stop);
                    }
                    session.approval_tx = None;
                }
            }
            debug_log(start, "Breaking out of main loop");
            break;
        }

        let new_title = format_window_title(&tab_manager);
        if new_title != last_title {
            title_manager.set_title(&new_title);
            last_title = new_title;
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

        // Check workflow completions and collect resumable sessions
        let completed = check_workflow_completions(
            &mut tab_manager,
            &working_dir,
            &workflow_config,
            &output_tx,
        )
        .await;
        resumable_sessions.extend(completed);
    }

    debug_log(start, "Loop exited, starting cleanup");

    // Cancel the periodic snapshot task
    let _ = snapshot_cancel_tx.send(());
    debug_log(start, "Snapshot task cancelled");

    // Abort any active workflow handles (no need to wait - state is already saved)
    let mut abort_count = 0;
    for session in tab_manager.sessions_mut() {
        if let Some(handle) = session.workflow_handle.take() {
            handle.abort();
            abort_count += 1;
        }
    }
    debug_log(start, &format!("Aborted {} workflow handles", abort_count));

    debug_log(start, "Restoring title");
    title_manager.restore_title();
    debug_log(start, "Restoring terminal");
    restore_terminal(&mut terminal)?;
    debug_log(start, "Terminal restored");

    // Print resume commands for sessions that were successfully stopped
    if !resumable_sessions.is_empty() {
        println!();
        for session in &resumable_sessions {
            let cmd = build_resume_command(&session.session_id, &session.working_dir);
            println!("To resume '{}': {}", session.feature_name, cmd);
        }
    }

    debug_log(start, "run_tui complete");
    Ok(())
}

fn load_workflow_config(
    cli: &Cli,
    working_dir: &Path,
    start: std::time::Instant,
) -> WorkflowConfig {
    // --claude flag takes priority over any config file
    if cli.claude {
        debug_log(start, "Using Claude-only workflow config (--claude)");
        return WorkflowConfig::claude_only_config();
    }

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
