use crate::planning_paths;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub struct AgentLogger {
    agent_name: String,
    file: Arc<Mutex<std::fs::File>>,
}

impl AgentLogger {
    pub fn new(agent_name: &str, working_dir: &Path) -> Option<Self> {
        let file = get_log_file(working_dir)?;
        Some(Self {
            agent_name: agent_name.to_string(),
            file,
        })
    }

    pub fn log_line(&self, kind: &str, line: &str) {
        if let Ok(mut file) = self.file.lock() {
            let now = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(file, "[{}][{}][{}] {}", now, self.agent_name, kind, line);
            let _ = file.flush();
        }
    }
}

static LOG_FILE: OnceLock<Arc<Mutex<std::fs::File>>> = OnceLock::new();
static RUN_ID: OnceLock<String> = OnceLock::new();

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
