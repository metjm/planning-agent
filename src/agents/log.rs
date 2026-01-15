use crate::planning_paths;
use crate::session_logger::SessionLogger;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Agent logger that can use either the new SessionLogger or legacy file-based logging.
pub struct AgentLogger {
    agent_name: String,
    /// New session-based logger (preferred)
    session_logger: Option<Arc<SessionLogger>>,
    /// Legacy file-based logger (fallback)
    file: Option<Arc<Mutex<std::fs::File>>>,
}

impl AgentLogger {
    /// Creates a new AgentLogger with session-based logging.
    ///
    /// This is the preferred constructor for new code.
    #[allow(dead_code)]
    pub fn with_session_logger(agent_name: &str, session_logger: Arc<SessionLogger>) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            session_logger: Some(session_logger),
            file: None,
        }
    }

    /// Creates a new AgentLogger with legacy file-based logging.
    ///
    /// **DEPRECATED**: Use `with_session_logger()` for new code.
    /// This is kept for backward compatibility during transition.
    pub fn new(agent_name: &str, working_dir: &Path) -> Option<Self> {
        let file = get_log_file(working_dir)?;
        Some(Self {
            agent_name: agent_name.to_string(),
            session_logger: None,
            file: Some(file),
        })
    }

    /// Logs a line of agent output.
    pub fn log_line(&self, kind: &str, line: &str) {
        // Prefer session logger if available
        if let Some(ref logger) = self.session_logger {
            logger.log_agent_stream(&self.agent_name, kind, line);
            return;
        }

        // Fall back to legacy file logging
        if let Some(ref file) = self.file {
            if let Ok(mut f) = file.lock() {
                let now = chrono::Local::now().format("%H:%M:%S%.3f");
                let _ = writeln!(f, "[{}][{}][{}] {}", now, self.agent_name, kind, line);
                let _ = f.flush();
            }
        }
    }
}

static LOG_FILE: OnceLock<Arc<Mutex<std::fs::File>>> = OnceLock::new();
static RUN_ID: OnceLock<String> = OnceLock::new();

/// Gets the legacy log file handle.
///
/// **DEPRECATED**: Use SessionLogger for new code.
fn get_log_file(working_dir: &Path) -> Option<Arc<Mutex<std::fs::File>>> {
    if let Some(file) = LOG_FILE.get() {
        return Some(file.clone());
    }

    let run_id = RUN_ID
        .get_or_init(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string())
        .clone();

    // Use home-based log path
    let log_path = planning_paths::agent_stream_log_path(working_dir, &run_id).ok()?;

    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(mut file) => {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(
                file,
                "\n=== Agent stream log started at {} (run {}) ===",
                now, run_id
            );
            let arc = Arc::new(Mutex::new(file));
            if LOG_FILE.set(arc.clone()).is_err() {
                return LOG_FILE.get().cloned();
            }
            Some(arc)
        }
        Err(_) => None,
    }
}
