mod events;
mod input;
mod input_naming;
mod session_events;
pub mod slash_commands;
pub mod snapshot_helper;
mod workflow_lifecycle;
mod workflow_loading;

use super::cli_usage;
use crate::app::cli::Cli;
use crate::app::util::{
    build_resume_command, debug_log, extract_feature_name, format_window_title,
};
// Re-export for submodules
pub(crate) use crate::app::workflow::run_workflow_with_config;
use crate::domain::input::{NewWorkflowInput, WorkflowInput};
use crate::domain::types::WorktreeState;
use crate::domain::view::WorkflowView;
use crate::planning_paths;
use crate::tui::{
    Event, EventHandler, InputMode, SessionStatus, TabManager, TerminalTitleManager,
    WorkflowCommand,
};
use crate::update;
use anyhow::{Context, Result};
use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use std::time::Duration;

pub use events::process_event;
pub use workflow_lifecycle::{check_workflow_completions, handle_init_completion, InitResult};
pub use workflow_loading::{restore_terminal, ResumableSession};

/// Handle to the initialization task for a new session.
/// Contains (session_id, join_handle) where join_handle resolves to InitResult.
pub type InitHandle = Option<(usize, tokio::task::JoinHandle<Result<InitResult>>)>;

/// Finds a session ID by feature name for --continue workflow support.
///
/// Searches through session_info.json files to find a session matching the feature name
/// that was started from the given working directory.
async fn find_session_by_feature_name(
    feature_name: &str,
    working_dir: &std::path::Path,
) -> Result<String> {
    let sessions_dir = planning_paths::sessions_dir()?;
    if !sessions_dir.exists() {
        anyhow::bail!(
            "No sessions found. Cannot continue workflow '{}'",
            feature_name
        );
    }

    let mut entries: Vec<_> = std::fs::read_dir(&sessions_dir)?
        .filter_map(|e| e.ok())
        .collect();

    // Sort by modification time (most recent first)
    entries.sort_by(|a, b| {
        let a_time = a.metadata().and_then(|m| m.modified()).ok();
        let b_time = b.metadata().and_then(|m| m.modified()).ok();
        b_time.cmp(&a_time)
    });

    for entry in entries {
        let session_id = entry.file_name().to_string_lossy().to_string();
        let info_path = sessions_dir.join(&session_id).join("session_info.json");
        if !info_path.exists() {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&info_path) {
            if let Ok(info) = serde_json::from_str::<serde_json::Value>(&content) {
                let matches_name = info
                    .get("feature_name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|n| n == feature_name);

                let matches_dir = info
                    .get("working_dir")
                    .and_then(|v| v.as_str())
                    .is_some_and(|d| std::path::Path::new(d) == working_dir);

                if matches_name && matches_dir {
                    return Ok(session_id);
                }
            }
        }
    }

    anyhow::bail!(
        "No session found for feature '{}' in directory '{}'",
        feature_name,
        working_dir.display()
    )
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

    // Set up panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Best-effort terminal restoration
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            crossterm::event::DisableBracketedPaste,
            crossterm::cursor::Show
        );
        // Call the original panic hook
        original_hook(panic_info);
    }));

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
    // keyboard_enhancement_enabled is informational only - no action needed if enhancement fails
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
    let mut scroll_regions = crate::tui::ScrollableRegions::new();
    debug_log(start, "tab manager created");
    let mut event_handler = EventHandler::new(Duration::from_millis(100));
    debug_log(start, "event handler created");
    let output_tx = event_handler.sender();

    // Set up signal handlers for graceful shutdown
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to create SIGTERM handler");

    #[cfg(unix)]
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .expect("Failed to create SIGINT handler");

    debug_log(start, "signal handlers created");

    {
        let usage_tx = event_handler.sender();
        tokio::spawn(async move {
            loop {
                let usage = tokio::task::spawn_blocking(cli_usage::fetch_all_provider_usage_sync)
                    .await
                    .unwrap_or_else(|_| cli_usage::AccountUsage::default());
                // Receiver dropped means TUI is shutting down - safe to ignore
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
                .unwrap_or_else(|_| update::UpdateStatus::CheckFailed("Task panicked".to_string()));
            // Receiver dropped means TUI is shutting down - safe to ignore
            let _ = update_tx.send(Event::UpdateStatusReceived(status));
        });
        debug_log(start, "update check task spawned");
    }

    // Spawn background version info fetch task
    {
        let version_tx = event_handler.sender();
        tokio::spawn(async move {
            let version_info =
                tokio::task::spawn_blocking(update::get_cached_or_fetch_version_info)
                    .await
                    .unwrap_or(None);
            // Receiver dropped means TUI is shutting down - safe to ignore
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
            // Receiver dropped means TUI is shutting down - safe to ignore
            let _ = file_index_tx.send(Event::FileIndexReady(index));
        });
        debug_log(start, "file index task spawned");
    }

    // Spawn daemon in background before starting subscription task
    // This ensures daemon is available when subscription tries to connect
    {
        let daemon_spawn_tx = event_handler.sender();
        tokio::spawn(async move {
            // Connect to daemon using new RPC client
            let client = crate::session_daemon::RpcClient::new(false).await;
            let connected = client.is_connected();

            // Send initial status if daemon was already running
            if connected {
                // Receiver dropped means TUI is shutting down - safe to ignore
                let _ = daemon_spawn_tx.send(Event::DaemonReconnected);
            }
        });
        debug_log(start, "daemon spawn task started");
    }

    // Spawn daemon subscription task for push notifications
    {
        let daemon_tx = event_handler.sender();
        tokio::spawn(async move {
            use crate::session_daemon::rpc_subscription::{RpcSubscription, SubscriptionEvent};

            crate::daemon_log::daemon_log("tui_runner", "subscription task started");
            let mut consecutive_failures: u32 = 0;
            loop {
                crate::daemon_log::daemon_log("tui_runner", "attempting to connect...");

                // After 3 consecutive failures, try to spawn the daemon
                // This handles the case where daemon died after initial startup
                if consecutive_failures >= 3 {
                    crate::daemon_log::daemon_log(
                        "tui_runner",
                        "3+ consecutive failures, attempting to spawn daemon",
                    );
                    // RpcClient::new will spawn daemon if not running
                    // Connection result is informational - we'll retry regardless
                    let _ = crate::session_daemon::RpcClient::new(false).await;
                    consecutive_failures = 0;
                }

                // Try to connect and subscribe via tarpc
                if let Some(mut subscription) = RpcSubscription::connect().await {
                    consecutive_failures = 0;
                    crate::daemon_log::daemon_log(
                        "tui_runner",
                        "connected! sending DaemonReconnected event",
                    );
                    // Receiver dropped means TUI is shutting down - safe to ignore
                    let _ = daemon_tx.send(Event::DaemonReconnected);

                    // Forward all push notifications to event system
                    crate::daemon_log::daemon_log("tui_runner", "entering recv loop");
                    while let Some(event) = subscription.recv().await {
                        crate::daemon_log::daemon_log(
                            "tui_runner",
                            &format!("received event: {:?}", event),
                        );
                        match event {
                            SubscriptionEvent::SessionChanged(record) => {
                                crate::daemon_log::daemon_log(
                                    "tui_runner",
                                    "forwarding SessionChanged event",
                                );
                                // Receiver dropped means TUI is shutting down - safe to ignore
                                let _ = daemon_tx.send(Event::DaemonSessionChanged(*record));
                            }
                            SubscriptionEvent::DaemonRestarting => {
                                crate::daemon_log::daemon_log(
                                    "tui_runner",
                                    "daemon is restarting, breaking recv loop",
                                );
                                break;
                            }
                            SubscriptionEvent::WorkflowEvent { session_id, event } => {
                                // CQRS workflow events - logged for debugging, not yet used in UI
                                crate::daemon_log::daemon_log(
                                    "tui_runner",
                                    &format!(
                                        "received workflow event for {}: {:?}",
                                        session_id, event
                                    ),
                                );
                            }
                        }
                    }
                    crate::daemon_log::daemon_log(
                        "tui_runner",
                        "recv loop ended, sending DaemonDisconnected event",
                    );
                    // Receiver dropped means TUI is shutting down - safe to ignore
                    let _ = daemon_tx.send(Event::DaemonDisconnected);
                } else {
                    consecutive_failures += 1;
                    crate::daemon_log::daemon_log(
                        "tui_runner",
                        &format!(
                            "connect() returned None (failure #{})",
                            consecutive_failures
                        ),
                    );
                }

                crate::daemon_log::daemon_log("tui_runner", "waiting 500ms before retry...");
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

    if update::consume_update_marker() {
        tab_manager.update_notice = Some("Update installed successfully!".to_string());
        debug_log(start, "update-installed marker consumed");
    }

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
        let snapshot = match crate::session_daemon::load_snapshot(session_id) {
            Ok(s) => s,
            Err(e) => {
                restore_terminal(&mut terminal)?;
                anyhow::bail!("Failed to load session '{}': {}", session_id, e);
            }
        };

        // Restore the session from snapshot using workflow_view if available
        let first_session = tab_manager.active_mut();

        // Extract workflow info from workflow_view
        let view = &snapshot.workflow_view;
        let feature_name = view
            .feature_name()
            .map(|n| n.0.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let phase_str = view
            .planning_phase()
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "Unknown".to_string());
        let iteration = view.iteration().map(|i| i.0).unwrap_or(1);

        *first_session = crate::tui::Session::from_ui_state(
            snapshot.ui_state.clone(),
            Some(snapshot.workflow_view.clone()),
        );
        first_session.name = feature_name.clone();
        first_session.add_output(format!("[planning] Resumed session: {}", session_id));
        first_session.add_output(format!(
            "[planning] Feature: {}, Phase: {}, Iteration: {}",
            feature_name, phase_str, iteration
        ));
        first_session.add_output("[planning] Continuing workflow...".to_string());

        // Restore elapsed time and cost from previous resume cycles
        first_session.total_cost = snapshot.ui_state.total_cost;
        first_session
            .adjust_start_time_for_previous_elapsed(snapshot.total_elapsed_before_resume_ms);

        // Load workflow config for resume.
        // Respects CLI overrides then snapshot's stored workflow.
        // This ensures the resumed session uses the same workflow that was originally used,
        // unless explicitly overridden by CLI flags.
        let resume_workflow_config =
            workflow_loading::load_workflow_config_for_resume(&cli, &snapshot, start);

        // Create WorkflowInput for resuming
        let workflow_input = match WorkflowInput::resume(session_id) {
            Ok(input) => input,
            Err(e) => {
                restore_terminal(&mut terminal)?;
                anyhow::bail!("Invalid session ID '{}': {}", session_id, e);
            }
        };

        // Get worktree info from workflow_view
        let worktree_info = snapshot.workflow_view.worktree_info().cloned();

        // Set up session context BEFORE starting the workflow
        // This enables proper working directory tracking for cross-directory resume
        let state_path = snapshot.state_path.clone();
        let context = crate::tui::SessionContext::from_snapshot(
            snapshot.working_dir.clone(),
            state_path.clone(),
            worktree_info.as_ref(),
            resume_workflow_config.clone(),
        );
        first_session.context = Some(context);

        // Build the initial view for the resumed workflow
        let initial_view = snapshot.workflow_view.clone();

        // Use the shared resume helper (same as /sessions overlay)
        workflow_lifecycle::start_resumed_workflow(
            first_session,
            workflow_input,
            initial_view,
            &working_dir,
            &resume_workflow_config,
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
        let init_session_id = first_session_id;

        let handle = tokio::spawn(async move {
            // Receiver dropped means TUI is shutting down - safe to ignore
            let _ = init_tx.send(Event::Output("[planning] Initializing...".to_string()));

            let feature_name = if let Some(name) = init_name {
                name
            } else {
                extract_feature_name(&init_objective, Some(&init_tx)).await?
            };

            // For --continue, find the session and resume it
            // For new workflows, create WorkflowInput::New
            if init_continue {
                // Receiver dropped means TUI is shutting down - safe to ignore
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Loading existing workflow: {}",
                    feature_name
                )));

                // Find existing session by feature name
                let session_id = find_session_by_feature_name(&feature_name, &init_working_dir)
                    .await
                    .context("Failed to find session for --continue")?;

                let state_path = planning_paths::session_state_path(&session_id)?;

                // Bootstrap view from event log
                let view = if let Ok(log_path) = planning_paths::session_event_log_path(&session_id)
                {
                    crate::domain::actor::bootstrap_view_from_events(&log_path, &session_id)
                } else {
                    WorkflowView::default()
                };

                // Check for existing worktree
                let effective_working_dir = if let Some(wt) = view.worktree_info() {
                    if crate::git_worktree::is_valid_worktree(wt.worktree_path()) {
                        // Receiver dropped means TUI is shutting down - safe to ignore
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Reusing existing worktree: {}",
                            wt.worktree_path().display()
                        )));
                        let _ = init_tx.send(Event::Output(format!(
                            "[planning] Branch: {}",
                            wt.branch_name()
                        )));
                        wt.worktree_path().to_path_buf()
                    } else {
                        // Receiver dropped means TUI is shutting down - safe to ignore
                        let _ = init_tx.send(Event::Output(
                            "[planning] Warning: Previous worktree no longer valid".to_string(),
                        ));
                        init_working_dir.clone()
                    }
                } else {
                    init_working_dir.clone()
                };

                // Send view update
                // Receiver dropped means TUI is shutting down - safe to ignore
                let _ = init_tx.send(Event::SessionViewUpdate {
                    session_id: init_session_id,
                    view: Box::new(view.clone()),
                });

                let workflow_input = WorkflowInput::resume(&session_id)
                    .map_err(|e| anyhow::anyhow!("Invalid session ID: {}", e))?;

                Ok::<_, anyhow::Error>(InitResult {
                    input: workflow_input,
                    view: Some(view),
                    state_path,
                    feature_name,
                    effective_working_dir,
                })
            } else {
                // Receiver dropped means TUI is shutting down - safe to ignore
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Starting new workflow: {}",
                    feature_name
                )));
                let _ = init_tx.send(Event::Output(format!(
                    "[planning] Objective: {}",
                    init_objective
                )));

                // Create new workflow input
                let mut new_input = NewWorkflowInput::new(
                    feature_name.clone(),
                    init_objective.clone(),
                    init_max_iterations,
                );

                // Generate a new workflow session ID
                let workflow_id = crate::domain::types::WorkflowId::new();
                let workflow_id_str = workflow_id.to_string();

                let state_path = planning_paths::session_state_path(&workflow_id_str)?;

                // Set up git worktree if enabled
                let effective_working_dir = if !worktree_flag {
                    init_working_dir.clone()
                } else {
                    let session_dir = match crate::planning_paths::session_dir(&workflow_id_str) {
                        Ok(dir) => dir,
                        Err(e) => {
                            // Receiver dropped means TUI is shutting down - safe to ignore
                            let _ = init_tx.send(Event::Output(format!(
                                "[planning] Warning: Could not get session directory: {}",
                                e
                            )));
                            let _ = init_tx.send(Event::Output(
                                "[planning] Continuing with original directory".to_string(),
                            ));

                            // View will be created via CQRS when WorkflowCreated event is emitted
                            return Ok::<_, anyhow::Error>(InitResult {
                                input: WorkflowInput::New(new_input),
                                view: None,
                                state_path,
                                feature_name,
                                effective_working_dir: init_working_dir.clone(),
                            });
                        }
                    };

                    let worktree_base = custom_worktree_dir
                        .as_ref()
                        .map(|d| d.to_path_buf())
                        .unwrap_or(session_dir);

                    match crate::git_worktree::create_session_worktree(
                        &init_working_dir,
                        &workflow_id_str,
                        &feature_name,
                        &worktree_base,
                        custom_worktree_branch.as_deref(),
                    ) {
                        crate::git_worktree::WorktreeSetupResult::Created(info) => {
                            // Receiver dropped means TUI is shutting down - safe to ignore
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

                            if info.has_submodules {
                                let _ = init_tx.send(Event::Output(
                                    "[planning] Warning: Repository has submodules".to_string(),
                                ));
                                let _ = init_tx.send(Event::Output(
                                    "[planning] Submodules may not be initialized in the worktree."
                                        .to_string(),
                                ));
                                let _ = init_tx.send(Event::Output(
                                    "[planning] Run 'git submodule update --init' in the worktree if needed.".to_string()
                                ));
                            }

                            let wt_state = WorktreeState::new(
                                info.worktree_path.clone(),
                                info.branch_name,
                                info.source_branch,
                                info.original_dir,
                            );
                            new_input = new_input.with_worktree(wt_state);
                            info.worktree_path
                        }
                        crate::git_worktree::WorktreeSetupResult::NotAGitRepo => {
                            // Receiver dropped means TUI is shutting down - safe to ignore
                            let _ = init_tx.send(Event::Output(
                                "[planning] Not a git repository, using original directory"
                                    .to_string(),
                            ));
                            init_working_dir.clone()
                        }
                        crate::git_worktree::WorktreeSetupResult::Failed(err) => {
                            // Receiver dropped means TUI is shutting down - safe to ignore
                            let _ = init_tx.send(Event::Output(format!(
                                "[planning] Warning: Git worktree setup failed: {}",
                                err
                            )));
                            let _ = init_tx.send(Event::Output(
                                "[planning] Continuing with original directory".to_string(),
                            ));
                            init_working_dir.clone()
                        }
                    }
                };

                // View will be created via CQRS when WorkflowCreated event is emitted
                Ok::<_, anyhow::Error>(InitResult {
                    input: WorkflowInput::New(new_input),
                    view: None,
                    state_path,
                    feature_name,
                    effective_working_dir,
                })
            }
        });
        debug_log(start, "init task spawned");

        init_handle = Some((first_session_id, handle));
    }
    let mut resumable_sessions: Vec<ResumableSession> = Vec::new();
    let mut quit_requested = false;

    debug_log(start, "entering main loop");

    const MAX_EVENTS_PER_FRAME: usize = 50;

    loop {
        terminal.draw(|frame| crate::tui::ui::draw(frame, &tab_manager, &mut scroll_regions))?;

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
                &scroll_regions,
                &mut terminal,
                &output_tx,
                &working_dir,
                &cli,
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
            use std::pin::Pin;
            use std::task::Poll;

            let signal_received = std::future::poll_fn(|cx| {
                if Pin::new(&mut sigterm).poll_recv(cx).is_ready() {
                    return Poll::Ready(true);
                }
                if Pin::new(&mut sigint).poll_recv(cx).is_ready() {
                    return Poll::Ready(true);
                }
                Poll::Ready(false)
            })
            .await;

            if signal_received {
                debug_log(start, "Signal received");
                quit_requested = true;
            }
        }

        // Handle quit: save state and exit immediately
        if quit_requested {
            debug_log(start, "Quit requested, saving snapshots");
            for session in tab_manager.sessions_mut() {
                // Save snapshot if we have workflow view
                if let Some(ref view) = session.workflow_view {
                    let session_id = view
                        .workflow_id()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    debug_log(
                        start,
                        &format!("Saving snapshot for session {}", session_id),
                    );
                    // Use session context's base_working_dir if available (preserves original session directory)
                    let session_working_dir = session
                        .context
                        .as_ref()
                        .map(|ctx| ctx.base_working_dir.clone())
                        .unwrap_or_else(|| working_dir.clone());
                    if let Err(e) = snapshot_helper::create_and_save_snapshot(
                        session,
                        view,
                        &session_working_dir,
                    ) {
                        debug_log(start, &format!("Failed to save snapshot: {}", e));
                    } else {
                        debug_log(start, "Snapshot saved successfully");
                        let feature_name = view
                            .feature_name()
                            .map(|n| n.0.clone())
                            .unwrap_or_else(|| "unknown".to_string());
                        resumable_sessions.push(ResumableSession::new(
                            feature_name,
                            session_id,
                            session_working_dir,
                        ));
                    }
                }
                // Send stop command and drop channels to unblock workflows
                if session.workflow_handle.is_some() {
                    if let Some(ref tx) = session.workflow_control_tx {
                        // Workflow may already be stopped or channel full - either case is acceptable during shutdown
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
                    &output_tx,
                )
                .await;
            } else {
                init_handle = Some((session_id, handle));
            }
        }

        // Check workflow completions and collect resumable sessions
        let completed =
            check_workflow_completions(&mut tab_manager, &working_dir, &output_tx).await;
        resumable_sessions.extend(completed);
    }

    debug_log(start, "Loop exited, starting cleanup");

    // Cancel the periodic snapshot task
    // Receiver dropped means task already exited - safe to ignore
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
