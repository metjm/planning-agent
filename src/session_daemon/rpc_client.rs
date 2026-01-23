//! RPC client for session daemon using tarpc.
//!
//! This module provides a tarpc-based client that replaces the old JSON-over-socket
//! implementation. Uses TCP on all platforms for consistency.

use crate::daemon_log::daemon_log;
use crate::planning_paths;
use crate::rpc::daemon_service::DaemonServiceClient;
use crate::rpc::{DaemonError, PortFileContent, SessionRecord};
use anyhow::{Context, Result};
use fs2::FileExt;
use std::sync::Arc;
use std::time::Duration;
use tarpc::client;
use tarpc::tokio_serde::formats::Bincode;
use tokio::sync::Mutex;

/// Maximum connection attempts with exponential backoff.
const MAX_CONNECT_ATTEMPTS: u32 = 5;

/// Base delay for exponential backoff (milliseconds).
const BASE_DELAY_MS: u64 = 100;

/// Maximum time to wait for daemon to initialize after spawning (milliseconds).
const DAEMON_INIT_TIMEOUT_MS: u64 = 2000;

/// RPC client for the session daemon.
///
/// Uses tarpc over TCP on all platforms.
pub struct RpcClient {
    /// The tarpc client
    inner: Arc<Mutex<Option<ClientState>>>,
    /// Whether we're in degraded mode (no daemon)
    degraded: bool,
}

struct ClientState {
    client: DaemonServiceClient,
    auth_token: String,
    authenticated: bool,
}

impl RpcClient {
    /// Creates a new RPC client, connecting to or spawning the daemon.
    ///
    /// If `no_daemon` is true, returns a degraded-mode client that doesn't
    /// connect to any daemon.
    pub async fn new(no_daemon: bool) -> Self {
        if no_daemon {
            return Self {
                inner: Arc::new(Mutex::new(None)),
                degraded: true,
            };
        }

        // Try to connect (errors are silent - daemon status shown in footer)
        match Self::connect_or_spawn().await {
            Ok(state) => Self {
                inner: Arc::new(Mutex::new(Some(state))),
                degraded: false,
            },
            Err(e) => {
                daemon_log("rpc_client", &format!("Failed to connect: {}", e));
                Self {
                    inner: Arc::new(Mutex::new(None)),
                    degraded: true,
                }
            }
        }
    }

    /// Creates a new RPC client synchronously (blocking).
    ///
    /// This is provided for compatibility with code that needs synchronous initialization.
    pub fn new_blocking(no_daemon: bool) -> Self {
        if no_daemon {
            return Self {
                inner: Arc::new(Mutex::new(None)),
                degraded: true,
            };
        }

        // Run async connect in a blocking context
        let result = std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");
            rt.block_on(Self::connect_or_spawn())
        })
        .join();

        match result {
            Ok(Ok(state)) => Self {
                inner: Arc::new(Mutex::new(Some(state))),
                degraded: false,
            },
            Ok(Err(e)) => {
                daemon_log("rpc_client", &format!("Failed to connect: {}", e));
                Self {
                    inner: Arc::new(Mutex::new(None)),
                    degraded: true,
                }
            }
            Err(_) => Self {
                inner: Arc::new(Mutex::new(None)),
                degraded: true,
            },
        }
    }

    /// Returns true if connected to daemon.
    pub fn is_connected(&self) -> bool {
        !self.degraded
    }

    /// Registers a session with the daemon.
    pub async fn register(&self, record: SessionRecord) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        match state
            .client
            .register(tarpc::context::current(), record)
            .await?
        {
            Ok(sha) => Ok(sha),
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Updates a session in the daemon.
    pub async fn update(&self, record: SessionRecord) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        match state
            .client
            .update(tarpc::context::current(), record)
            .await?
        {
            Ok(sha) => Ok(sha),
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Sends a heartbeat for a session.
    pub async fn heartbeat(&self, session_id: &str) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        match state
            .client
            .heartbeat(tarpc::context::current(), session_id.to_string())
            .await?
        {
            Ok(()) => {
                // Get build SHA
                let sha = state.client.build_sha(tarpc::context::current()).await?;
                Ok(sha)
            }
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Lists all sessions from the daemon.
    pub async fn list(&self) -> Result<Vec<SessionRecord>> {
        if self.degraded {
            return Ok(Vec::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        match state.client.list(tarpc::context::current()).await? {
            Ok(sessions) => Ok(sessions),
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Force-stops a session.
    pub async fn force_stop(&self, session_id: &str) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        match state
            .client
            .force_stop(tarpc::context::current(), session_id.to_string())
            .await?
        {
            Ok(()) => {
                let sha = state.client.build_sha(tarpc::context::current()).await?;
                Ok(sha)
            }
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Requests daemon shutdown (for updates).
    pub async fn shutdown(&self) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut guard = self.inner.lock().await;
        let state = guard.as_mut().context("Not connected to daemon")?;

        self.ensure_authenticated(state).await?;

        // Get SHA before shutdown
        let sha = state.client.build_sha(tarpc::context::current()).await?;

        match state.client.shutdown(tarpc::context::current()).await? {
            Ok(()) => Ok(sha),
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Attempts to reconnect to the daemon.
    pub async fn reconnect(&mut self) -> Result<()> {
        match Self::connect_or_spawn().await {
            Ok(state) => {
                let mut guard = self.inner.lock().await;
                *guard = Some(state);
                self.degraded = false;
                Ok(())
            }
            Err(e) => {
                self.degraded = true;
                Err(e)
            }
        }
    }

    /// Ensure the client is authenticated before making RPC calls.
    async fn ensure_authenticated(&self, state: &mut ClientState) -> Result<()> {
        if state.authenticated {
            return Ok(());
        }

        match state
            .client
            .authenticate(tarpc::context::current(), state.auth_token.clone())
            .await?
        {
            Ok(()) => {
                state.authenticated = true;
                Ok(())
            }
            Err(DaemonError::AuthenticationFailed) => {
                anyhow::bail!("Authentication failed")
            }
            Err(e) => anyhow::bail!("Daemon error: {}", e),
        }
    }

    /// Connect to daemon or spawn it if not running.
    async fn connect_or_spawn() -> Result<ClientState> {
        let port_path = planning_paths::sessiond_port_path()?;

        // Try reading port file and connecting
        if port_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&port_path) {
                if let Ok(port_info) = serde_json::from_str::<PortFileContent>(&content) {
                    if let Ok(state) = Self::try_connect(&port_info).await {
                        // Check if we should upgrade the daemon (only if we're newer)
                        let our_timestamp = crate::update::BUILD_TIMESTAMP;
                        let daemon_timestamp = state
                            .client
                            .build_timestamp(tarpc::context::current())
                            .await
                            .unwrap_or(0);

                        // Only request upgrade if timestamps differ and we have valid timestamps
                        if our_timestamp != daemon_timestamp
                            && our_timestamp > 0
                            && daemon_timestamp > 0
                        {
                            daemon_log(
                                "rpc_client",
                                &format!(
                                    "Version mismatch detected: client={}, daemon={}",
                                    our_timestamp, daemon_timestamp
                                ),
                            );

                            // Ask daemon if it will accept our upgrade request
                            // Daemon will only agree if we're newer
                            let upgrade_accepted = state
                                .client
                                .request_upgrade(tarpc::context::current(), our_timestamp)
                                .await
                                .unwrap_or(false);

                            if upgrade_accepted {
                                daemon_log(
                                    "rpc_client",
                                    "Daemon accepted upgrade request, waiting for shutdown",
                                );

                                // Wait for daemon to exit
                                tokio::time::sleep(Duration::from_millis(200)).await;

                                // Remove port file
                                let _ = std::fs::remove_file(&port_path);
                                // Fall through to spawn new daemon
                            } else {
                                daemon_log(
                                    "rpc_client",
                                    "Daemon refused upgrade (we're not newer), connecting normally",
                                );
                                return Ok(state);
                            }
                        } else {
                            return Ok(state);
                        }
                    }
                }
            }
        }

        // Need to spawn daemon - acquire lock
        let lock_path = planning_paths::sessiond_lock_path()?;
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .context("Failed to open lock file")?;

        // Try to acquire exclusive lock
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // We have the lock - spawn daemon
                Self::spawn_daemon_and_wait().await?;
                FileExt::unlock(&lock_file)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Another process is spawning - wait for daemon
                Self::wait_for_port_file(&port_path).await?;
            }
            Err(e) => {
                return Err(e).context("Failed to acquire lock");
            }
        }

        // Connect with retry
        Self::connect_with_retry(&port_path).await
    }

    /// Try to connect to daemon using port info.
    async fn try_connect(port_info: &PortFileContent) -> Result<ClientState> {
        use tarpc::serde_transport::tcp;

        let addr = format!("127.0.0.1:{}", port_info.port);
        let transport = tcp::connect(&addr, Bincode::default).await?;
        let client = DaemonServiceClient::new(client::Config::default(), transport).spawn();

        Ok(ClientState {
            client,
            auth_token: port_info.token.clone(),
            authenticated: false,
        })
    }

    /// Spawn daemon and wait for it to be ready.
    ///
    /// This is only called when connection to existing daemon failed,
    /// so we aggressively kill any old process and start fresh.
    async fn spawn_daemon_and_wait() -> Result<()> {
        let pid_path = planning_paths::sessiond_pid_path()?;
        let port_path = planning_paths::sessiond_port_path()?;

        // Kill any existing daemon process (may be zombie or unresponsive)
        if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                daemon_log("rpc_client", &format!("Killing old daemon process {}", pid));
                #[cfg(unix)]
                {
                    // SIGKILL to ensure it dies (SIGTERM might be ignored by zombie)
                    unsafe {
                        nix::libc::kill(pid, nix::libc::SIGKILL);
                    }
                }
                #[cfg(windows)]
                {
                    use windows_sys::Win32::Foundation::CloseHandle;
                    use windows_sys::Win32::System::Threading::{
                        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
                    };
                    unsafe {
                        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
                        if handle != 0 {
                            TerminateProcess(handle, 1);
                            CloseHandle(handle);
                        }
                    }
                }
                // Give OS time to clean up
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        // Clean up stale files
        let _ = std::fs::remove_file(&pid_path);
        let _ = std::fs::remove_file(&port_path);
        if let Ok(sha_path) = planning_paths::sessiond_build_sha_path() {
            let _ = std::fs::remove_file(&sha_path);
        }

        daemon_log("rpc_client", "Spawning new daemon");

        // Spawn daemon
        let exe = std::env::current_exe()
            .or_else(|_| which::which("planning"))
            .context("Failed to find planning binary")?;

        // Get current home directory and pass to daemon so it uses the same path.
        // This is essential for test isolation where tests set a custom home dir.
        let home_dir = planning_paths::planning_agent_home_dir()?;

        #[cfg(unix)]
        {
            std::process::Command::new(&exe)
                .arg("--session-daemon")
                .env("PLANNING_AGENT_HOME", &home_dir)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("Failed to spawn daemon")?;
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std::process::Command::new(&exe)
                .arg("--session-daemon")
                .env("PLANNING_AGENT_HOME", &home_dir)
                .creation_flags(0x00000008) // DETACHED_PROCESS
                .spawn()
                .context("Failed to spawn daemon")?;
        }

        // Wait for port file
        Self::wait_for_port_file(&port_path).await
    }

    /// Wait for port file to appear.
    async fn wait_for_port_file(port_path: &std::path::Path) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(DAEMON_INIT_TIMEOUT_MS);

        while start.elapsed() < timeout {
            if port_path.exists() {
                // Verify we can read and parse it
                if let Ok(content) = std::fs::read_to_string(port_path) {
                    if serde_json::from_str::<PortFileContent>(&content).is_ok() {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        anyhow::bail!("Timeout waiting for daemon port file")
    }

    /// Connect with retry and exponential backoff.
    async fn connect_with_retry(port_path: &std::path::Path) -> Result<ClientState> {
        let mut delay_ms = BASE_DELAY_MS;

        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            if let Ok(content) = std::fs::read_to_string(port_path) {
                if let Ok(port_info) = serde_json::from_str::<PortFileContent>(&content) {
                    if let Ok(state) = Self::try_connect(&port_info).await {
                        return Ok(state);
                    }
                }
            }

            if attempt < MAX_CONNECT_ATTEMPTS {
                // Add jitter (±25%)
                let jitter = (delay_ms as f64 * 0.25 * (rand::random::<f64>() * 2.0 - 1.0)) as i64;
                let actual_delay = (delay_ms as i64 + jitter).max(10) as u64;
                tokio::time::sleep(Duration::from_millis(actual_delay)).await;
                delay_ms *= 2;
            }
        }

        anyhow::bail!("Failed to connect after {} attempts", MAX_CONNECT_ATTEMPTS)
    }
}
