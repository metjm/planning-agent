//! Session daemon client implementation.
//!
//! Provides the connect-or-spawn pattern for connecting to the session daemon
//! and automatically spawning it if not running.

#![allow(dead_code)]

use crate::planning_paths;
use crate::session_daemon::protocol::{ClientMessage, DaemonMessage, SessionRecord};
use anyhow::{Context, Result};
use fs2::FileExt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Maximum connection attempts with exponential backoff.
const MAX_CONNECT_ATTEMPTS: u32 = 5;

/// Base delay for exponential backoff (milliseconds).
const BASE_DELAY_MS: u64 = 100;

/// Maximum time to wait for daemon to initialize after PID file exists (milliseconds).
const DAEMON_INIT_TIMEOUT_MS: u64 = 2000;

/// Session daemon client.
///
/// Manages connection to the daemon with automatic reconnection.
pub struct SessionDaemonClient {
    /// Active connection (if any)
    #[cfg(unix)]
    connection: Arc<Mutex<Option<UnixConnection>>>,

    #[cfg(windows)]
    connection: Arc<Mutex<Option<TcpConnection>>>,

    /// Whether we're in degraded mode (no daemon)
    degraded: bool,
}

#[cfg(unix)]
struct UnixConnection {
    reader: BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(windows)]
struct TcpConnection {
    reader: BufReader<std::net::TcpStream>,
    writer: std::net::TcpStream,
}

impl SessionDaemonClient {
    /// Creates a new client, connecting to or spawning the daemon.
    ///
    /// If `no_daemon` is true, returns a degraded-mode client that doesn't
    /// connect to any daemon.
    pub fn new(no_daemon: bool) -> Self {
        if no_daemon {
            return Self {
                connection: Arc::new(Mutex::new(None)),
                degraded: true,
            };
        }

        // Try to connect (errors are silent - daemon status shown in footer)
        let (connection, degraded) = match Self::connect_or_spawn() {
            Ok(conn) => (Some(conn), false),
            Err(_) => (None, true),
        };

        Self {
            connection: Arc::new(Mutex::new(connection)),
            degraded,
        }
    }

    /// Returns true if connected to daemon.
    pub fn is_connected(&self) -> bool {
        !self.degraded
    }

    /// Registers a session with the daemon.
    pub async fn register(&self, record: SessionRecord) -> Result<String> {
        self.send_message(ClientMessage::Register(record)).await
    }

    /// Updates a session in the daemon.
    pub async fn update(&self, record: SessionRecord) -> Result<String> {
        self.send_message(ClientMessage::Update(record)).await
    }

    /// Sends a heartbeat for a session.
    pub async fn heartbeat(&self, session_id: &str) -> Result<String> {
        self.send_message(ClientMessage::Heartbeat {
            session_id: session_id.to_string(),
        })
        .await
    }

    /// Lists all sessions from the daemon.
    pub async fn list(&self) -> Result<Vec<SessionRecord>> {
        if self.degraded {
            return Ok(Vec::new());
        }

        let mut conn_guard = self.connection.lock().await;
        let conn = conn_guard
            .as_mut()
            .context("Not connected to daemon")?;

        let msg = serde_json::to_string(&ClientMessage::List)?;

        #[cfg(unix)]
        {
            writeln!(conn.writer, "{}", msg)?;
            conn.writer.flush()?;

            let mut response_line = String::new();
            conn.reader.read_line(&mut response_line)?;

            let response: DaemonMessage = serde_json::from_str(response_line.trim())?;
            match response {
                DaemonMessage::Sessions(sessions) => Ok(sessions),
                DaemonMessage::Error(e) => anyhow::bail!("Daemon error: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }

        #[cfg(windows)]
        {
            writeln!(conn.writer, "{}", msg)?;
            conn.writer.flush()?;

            let mut response_line = String::new();
            conn.reader.read_line(&mut response_line)?;

            let response: DaemonMessage = serde_json::from_str(response_line.trim())?;
            match response {
                DaemonMessage::Sessions(sessions) => Ok(sessions),
                DaemonMessage::Error(e) => anyhow::bail!("Daemon error: {}", e),
                _ => anyhow::bail!("Unexpected response"),
            }
        }
    }

    /// Force-stops a session.
    pub async fn force_stop(&self, session_id: &str) -> Result<String> {
        self.send_message(ClientMessage::ForceStop {
            session_id: session_id.to_string(),
        })
        .await
    }

    /// Requests daemon shutdown (for updates).
    pub async fn shutdown(&self) -> Result<String> {
        self.send_message(ClientMessage::Shutdown).await
    }

    /// Attempts to reconnect to the daemon.
    ///
    /// This will try to connect to an existing daemon or spawn a new one.
    /// If successful, clears the degraded state.
    pub async fn reconnect(&mut self) -> Result<()> {
        match Self::connect_or_spawn() {
            Ok(connection) => {
                let mut conn_guard = self.connection.lock().await;
                *conn_guard = Some(connection);
                self.degraded = false;
                Ok(())
            }
            Err(e) => {
                self.degraded = true;
                Err(e)
            }
        }
    }

    /// Sends a message and returns the build SHA from the Ack response.
    async fn send_message(&self, message: ClientMessage) -> Result<String> {
        if self.degraded {
            return Ok(String::new());
        }

        let mut conn_guard = self.connection.lock().await;
        let conn = conn_guard
            .as_mut()
            .context("Not connected to daemon")?;

        let msg = serde_json::to_string(&message)?;

        #[cfg(unix)]
        {
            writeln!(conn.writer, "{}", msg)?;
            conn.writer.flush()?;

            let mut response_line = String::new();
            conn.reader.read_line(&mut response_line)?;

            let response: DaemonMessage = serde_json::from_str(response_line.trim())?;
            match response {
                DaemonMessage::Ack { build_sha } => Ok(build_sha),
                DaemonMessage::Error(e) => anyhow::bail!("Daemon error: {}", e),
                DaemonMessage::Restarting { new_sha } => {
                    anyhow::bail!("Daemon restarting to version {}", new_sha)
                }
                _ => anyhow::bail!("Unexpected response"),
            }
        }

        #[cfg(windows)]
        {
            writeln!(conn.writer, "{}", msg)?;
            conn.writer.flush()?;

            let mut response_line = String::new();
            conn.reader.read_line(&mut response_line)?;

            let response: DaemonMessage = serde_json::from_str(response_line.trim())?;
            match response {
                DaemonMessage::Ack { build_sha } => Ok(build_sha),
                DaemonMessage::Error(e) => anyhow::bail!("Daemon error: {}", e),
                DaemonMessage::Restarting { new_sha } => {
                    anyhow::bail!("Daemon restarting to version {}", new_sha)
                }
                _ => anyhow::bail!("Unexpected response"),
            }
        }
    }

    /// Connect to daemon or spawn it if not running.
    #[cfg(unix)]
    fn connect_or_spawn() -> Result<UnixConnection> {
        use std::os::unix::net::UnixStream;

        let socket_path = planning_paths::sessiond_socket_path()?;

        // Try connecting first
        if let Ok(stream) = UnixStream::connect(&socket_path) {
            let writer = stream.try_clone()?;
            let mut conn = UnixConnection {
                reader: BufReader::new(stream),
                writer,
            };

            // Check version mismatch - send a List and check build_sha
            if let Some(daemon_sha) = Self::get_daemon_build_sha_unix(&mut conn) {
                let our_sha = crate::update::BUILD_SHA;
                if daemon_sha != our_sha && our_sha != "unknown" && daemon_sha != "unknown" {
                    // Version mismatch - shutdown old daemon and spawn new one (silent)
                    let _ = Self::send_shutdown_unix(&mut conn);
                    // Wait for daemon to exit
                    std::thread::sleep(Duration::from_millis(200));
                    // Remove stale socket
                    let _ = std::fs::remove_file(&socket_path);
                    // Fall through to spawn new daemon
                } else {
                    return Ok(conn);
                }
            } else {
                return Ok(conn);
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
                Self::spawn_daemon_and_wait(&socket_path)?;
                lock_file.unlock()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Another process is spawning - wait for daemon
                Self::wait_for_daemon(&socket_path)?;
            }
            Err(e) => {
                return Err(e).context("Failed to acquire lock");
            }
        }

        // Connect with retry
        Self::connect_with_retry(&socket_path)
    }

    #[cfg(unix)]
    fn spawn_daemon_and_wait(socket_path: &PathBuf) -> Result<()> {
        let pid_path = planning_paths::sessiond_pid_path()?;

        // Check if daemon is already running (PID file exists and process alive)
        if pid_path.exists() {
            if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    // Check if process is alive
                    if unsafe { nix::libc::kill(pid, 0) } == 0 {
                        // Process is alive, wait for socket
                        Self::wait_for_socket(socket_path, DAEMON_INIT_TIMEOUT_MS)?;
                        return Ok(());
                    }
                }
            }
            // Stale PID file, remove it
            let _ = std::fs::remove_file(&pid_path);
        }

        // Remove stale socket if exists
        if socket_path.exists() {
            let _ = std::fs::remove_file(socket_path);
        }

        // Spawn daemon
        let exe = std::env::current_exe()
            .or_else(|_| which::which("planning"))
            .context("Failed to find planning binary")?;

        std::process::Command::new(&exe)
            .arg("--session-daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to spawn daemon")?;

        // Wait for socket to appear
        Self::wait_for_socket(socket_path, DAEMON_INIT_TIMEOUT_MS)?;

        Ok(())
    }

    #[cfg(unix)]
    fn wait_for_daemon(socket_path: &PathBuf) -> Result<()> {
        Self::wait_for_socket(socket_path, DAEMON_INIT_TIMEOUT_MS)
    }

    #[cfg(unix)]
    fn wait_for_socket(socket_path: &PathBuf, timeout_ms: u64) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            if socket_path.exists() {
                // Try connecting to verify it's ready
                if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        anyhow::bail!("Timeout waiting for daemon socket")
    }

    /// Get the daemon's build SHA by sending a Heartbeat and reading the Ack response.
    #[cfg(unix)]
    fn get_daemon_build_sha_unix(conn: &mut UnixConnection) -> Option<String> {
        // Send a heartbeat with empty session ID just to get the Ack
        let msg = serde_json::to_string(&ClientMessage::Heartbeat {
            session_id: String::new(),
        })
        .ok()?;
        writeln!(conn.writer, "{}", msg).ok()?;
        conn.writer.flush().ok()?;

        let mut response_line = String::new();
        conn.reader.read_line(&mut response_line).ok()?;

        let response: DaemonMessage = serde_json::from_str(response_line.trim()).ok()?;
        match response {
            DaemonMessage::Ack { build_sha } => Some(build_sha),
            _ => None,
        }
    }

    /// Send shutdown message to daemon (best effort).
    #[cfg(unix)]
    fn send_shutdown_unix(conn: &mut UnixConnection) -> Result<()> {
        let msg = serde_json::to_string(&ClientMessage::Shutdown)?;
        writeln!(conn.writer, "{}", msg)?;
        conn.writer.flush()?;
        Ok(())
    }

    #[cfg(unix)]
    fn connect_with_retry(socket_path: &PathBuf) -> Result<UnixConnection> {
        use rand::Rng;
        use std::os::unix::net::UnixStream;

        let mut delay_ms = BASE_DELAY_MS;
        let mut rng = rand::thread_rng();

        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            match UnixStream::connect(socket_path) {
                Ok(stream) => {
                    let writer = stream.try_clone()?;
                    return Ok(UnixConnection {
                        reader: BufReader::new(stream),
                        writer,
                    });
                }
                Err(e) if attempt < MAX_CONNECT_ATTEMPTS => {
                    // Add jitter (±25%)
                    let jitter = (delay_ms as f64 * 0.25 * (rng.gen::<f64>() * 2.0 - 1.0)) as u64;
                    let actual_delay = delay_ms.saturating_add_signed(jitter as i64);
                    std::thread::sleep(Duration::from_millis(actual_delay));
                    delay_ms *= 2;
                }
                Err(e) => {
                    return Err(e).context("Failed to connect after retries");
                }
            }
        }

        anyhow::bail!("Failed to connect after {} attempts", MAX_CONNECT_ATTEMPTS)
    }

    /// Connect to daemon or spawn it if not running (Windows).
    #[cfg(windows)]
    fn connect_or_spawn() -> Result<TcpConnection> {
        use crate::session_daemon::protocol::PortFileContent;
        use std::net::TcpStream;

        let port_path = planning_paths::sessiond_port_path()?;

        // Try reading port file and connecting
        if port_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&port_path) {
                if let Ok(port_info) = serde_json::from_str::<PortFileContent>(&content) {
                    let addr = format!("127.0.0.1:{}", port_info.port);
                    if let Ok(mut stream) = TcpStream::connect(&addr) {
                        // Send authentication token
                        writeln!(stream, "{}", port_info.token)?;
                        stream.flush()?;

                        let reader = BufReader::new(stream.try_clone()?);
                        let mut conn = TcpConnection {
                            reader,
                            writer: stream,
                        };

                        // Check version mismatch
                        if let Some(daemon_sha) = Self::get_daemon_build_sha_windows(&mut conn) {
                            let our_sha = crate::update::BUILD_SHA;
                            if daemon_sha != our_sha && our_sha != "unknown" && daemon_sha != "unknown" {
                                // Version mismatch - shutdown old daemon and spawn new one (silent)
                                let _ = Self::send_shutdown_windows(&mut conn);
                                // Wait for daemon to exit
                                std::thread::sleep(Duration::from_millis(200));
                                // Remove port file
                                let _ = std::fs::remove_file(&port_path);
                                // Fall through to spawn new daemon
                            } else {
                                return Ok(conn);
                            }
                        } else {
                            return Ok(conn);
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
                Self::spawn_daemon_windows()?;
                lock_file.unlock()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Another process is spawning - wait for daemon
                Self::wait_for_daemon_windows(&port_path)?;
            }
            Err(e) => {
                return Err(e).context("Failed to acquire lock");
            }
        }

        // Connect with retry
        Self::connect_with_retry_windows(&port_path)
    }

    #[cfg(windows)]
    fn spawn_daemon_windows() -> Result<()> {
        let exe = std::env::current_exe()
            .or_else(|_| which::which("planning"))
            .context("Failed to find planning binary")?;

        std::process::Command::new(&exe)
            .arg("--session-daemon")
            .creation_flags(0x00000008) // DETACHED_PROCESS
            .spawn()
            .context("Failed to spawn daemon")?;

        // Wait for port file
        let port_path = planning_paths::sessiond_port_path()?;
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(DAEMON_INIT_TIMEOUT_MS);

        while start.elapsed() < timeout {
            if port_path.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        anyhow::bail!("Timeout waiting for daemon")
    }

    #[cfg(windows)]
    fn wait_for_daemon_windows(port_path: &PathBuf) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(DAEMON_INIT_TIMEOUT_MS);

        while start.elapsed() < timeout {
            if port_path.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        anyhow::bail!("Timeout waiting for daemon")
    }

    /// Get the daemon's build SHA by sending a Heartbeat and reading the Ack response.
    #[cfg(windows)]
    fn get_daemon_build_sha_windows(conn: &mut TcpConnection) -> Option<String> {
        // Send a heartbeat with empty session ID just to get the Ack
        let msg = serde_json::to_string(&ClientMessage::Heartbeat {
            session_id: String::new(),
        })
        .ok()?;
        writeln!(conn.writer, "{}", msg).ok()?;
        conn.writer.flush().ok()?;

        let mut response_line = String::new();
        conn.reader.read_line(&mut response_line).ok()?;

        let response: DaemonMessage = serde_json::from_str(response_line.trim()).ok()?;
        match response {
            DaemonMessage::Ack { build_sha } => Some(build_sha),
            _ => None,
        }
    }

    /// Send shutdown message to daemon (best effort).
    #[cfg(windows)]
    fn send_shutdown_windows(conn: &mut TcpConnection) -> Result<()> {
        let msg = serde_json::to_string(&ClientMessage::Shutdown)?;
        writeln!(conn.writer, "{}", msg)?;
        conn.writer.flush()?;
        Ok(())
    }

    #[cfg(windows)]
    fn connect_with_retry_windows(port_path: &PathBuf) -> Result<TcpConnection> {
        use crate::session_daemon::protocol::PortFileContent;
        use rand::Rng;
        use std::net::TcpStream;

        let mut delay_ms = BASE_DELAY_MS;
        let mut rng = rand::thread_rng();

        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            if let Ok(content) = std::fs::read_to_string(port_path) {
                if let Ok(port_info) = serde_json::from_str::<PortFileContent>(&content) {
                    let addr = format!("127.0.0.1:{}", port_info.port);
                    if let Ok(mut stream) = TcpStream::connect(&addr) {
                        // Send authentication token
                        writeln!(stream, "{}", port_info.token)?;
                        stream.flush()?;

                        let reader = BufReader::new(stream.try_clone()?);
                        return Ok(TcpConnection {
                            reader,
                            writer: stream,
                        });
                    }
                }
            }

            if attempt < MAX_CONNECT_ATTEMPTS {
                // Add jitter (±25%)
                let jitter = (delay_ms as f64 * 0.25 * (rng.gen::<f64>() * 2.0 - 1.0)) as u64;
                let actual_delay = delay_ms.saturating_add_signed(jitter as i64);
                std::thread::sleep(Duration::from_millis(actual_delay));
                delay_ms *= 2;
            }
        }

        anyhow::bail!("Failed to connect after {} attempts", MAX_CONNECT_ATTEMPTS)
    }
}

/// Checks if a process with the given PID is alive.
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    unsafe { nix::libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use std::ptr::null_mut;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let result = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        result != 0 && exit_code == 259 // STILL_ACTIVE
    }
}

#[cfg(windows)]
extern "system" {
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> *mut std::ffi::c_void;
    fn GetExitCodeProcess(hProcess: *mut std::ffi::c_void, lpExitCode: *mut u32) -> i32;
    fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_process_alive_self() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn test_is_process_alive_nonexistent() {
        // Use a very high PID that's unlikely to exist
        assert!(!is_process_alive(999999999));
    }

    #[test]
    fn test_client_degraded_mode() {
        let client = SessionDaemonClient::new(true);
        assert!(!client.is_connected());
    }
}
