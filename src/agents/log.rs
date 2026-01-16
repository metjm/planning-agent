use crate::session_logger::SessionLogger;
use std::sync::Arc;

/// Agent logger that uses SessionLogger for all agent output logging.
pub struct AgentLogger {
    agent_name: String,
    session_logger: Arc<SessionLogger>,
}

impl AgentLogger {
    /// Creates a new AgentLogger with session-based logging.
    pub fn new(agent_name: &str, session_logger: Arc<SessionLogger>) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            session_logger,
        }
    }

    /// Logs a line of agent output.
    pub fn log_line(&self, kind: &str, line: &str) {
        self.session_logger.log_agent_stream(&self.agent_name, kind, line);
    }
}
