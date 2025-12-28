use crate::planning_dir::ensure_planning_agent_dir;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Generates a unique prefix for plan/feedback filenames using timestamp and UUID.
/// Format: YYYYMMDD-HHMMSS-xxxxxxxx (where xxxxxxxx is first 8 chars of UUID)
fn generate_unique_prefix() -> String {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let uuid_suffix = &Uuid::new_v4().to_string()[..8];
    format!("{}-{}", timestamp, uuid_suffix)
}

/// Generates a unique plan filename with timestamp prefix.
/// Format: YYYYMMDD-HHMMSS-xxxxxxxx_<sanitized_name>.md
fn generate_plan_filename(prefix: &str, sanitized_name: &str) -> PathBuf {
    PathBuf::from(format!("docs/plans/{}_{}.md", prefix, sanitized_name))
}

/// Generates a unique feedback filename with timestamp prefix and round number.
/// Format: YYYYMMDD-HHMMSS-xxxxxxxx_<sanitized_name>_feedback_<round>.md
fn generate_feedback_filename(prefix: &str, sanitized_name: &str, round: u32) -> PathBuf {
    PathBuf::from(format!(
        "docs/plans/{}_{}_feedback_{}.md",
        prefix, sanitized_name, round
    ))
}

/// Extracts the unique prefix from a plan filename (new format).
/// Returns None for legacy filenames without timestamp prefix.
fn extract_unique_prefix(plan_file: &Path) -> Option<String> {
    let filename = plan_file.file_stem()?.to_str()?;
    // New format: "YYYYMMDD-HHMMSS-xxxxxxxx_feature-name"
    // Split on first underscore to get prefix
    if let Some(underscore_pos) = filename.find('_') {
        let prefix = &filename[..underscore_pos];
        // Validate it looks like a timestamp-uuid prefix (pattern: NNNNNNNN-NNNNNN-xxxxxxxx)
        // Length should be at least 24 chars (8 + 1 + 6 + 1 + 8)
        if prefix.len() >= 24 && prefix.chars().nth(8) == Some('-') {
            return Some(prefix.to_string());
        }
    }
    None
}

/// Extracts the sanitized feature name from a plan filename.
/// Works with both new format (with prefix) and legacy format (without prefix).
fn extract_sanitized_name(plan_file: &Path) -> Option<String> {
    let filename = plan_file.file_stem()?.to_str()?;

    // Try new format first: "YYYYMMDD-HHMMSS-xxxxxxxx_feature-name"
    if let Some(underscore_pos) = filename.find('_') {
        let prefix = &filename[..underscore_pos];
        // Validate it looks like a timestamp-uuid prefix
        if prefix.len() >= 24 && prefix.chars().nth(8) == Some('-') {
            return Some(filename[underscore_pos + 1..].to_string());
        }
    }

    // Legacy format: just the feature name (e.g., "existing-feature")
    Some(filename.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Reviewing,
    Revising,
    Complete,
}

impl Phase {
    /// Get a UI-friendly label for the phase.
    #[allow(dead_code)]
    pub fn label(&self) -> PhaseLabel {
        match self {
            Phase::Planning => PhaseLabel::Planning,
            Phase::Reviewing => PhaseLabel::Reviewing,
            Phase::Revising => PhaseLabel::Revising,
            Phase::Complete => PhaseLabel::Complete,
        }
    }
}

/// Human-readable phase labels for UI/logging purposes.
///
/// Unlike `Phase`, which is used for state machine transitions,
/// `PhaseLabel` provides display-friendly formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PhaseLabel {
    Planning,
    Reviewing,
    Revising,
    Complete,
}

#[allow(dead_code)]
impl PhaseLabel {
    /// Short label for compact display (e.g., status bars).
    pub fn short(&self) -> &'static str {
        match self {
            PhaseLabel::Planning => "Plan",
            PhaseLabel::Reviewing => "Review",
            PhaseLabel::Revising => "Revise",
            PhaseLabel::Complete => "Done",
        }
    }

    /// Full label for verbose display.
    pub fn full(&self) -> &'static str {
        match self {
            PhaseLabel::Planning => "Planning",
            PhaseLabel::Reviewing => "Reviewing",
            PhaseLabel::Revising => "Revising",
            PhaseLabel::Complete => "Complete",
        }
    }

    /// Label with iteration number for review/revise phases.
    pub fn with_iteration(&self, iteration: u32) -> String {
        match self {
            PhaseLabel::Reviewing if iteration > 1 => format!("Reviewing #{}", iteration),
            PhaseLabel::Revising => format!("Revising #{}", iteration),
            _ => self.full().to_string(),
        }
    }
}

impl std::fmt::Display for PhaseLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Approved,
    NeedsRevision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResumeStrategy {
    #[default]
    Stateless,
    SessionId,
    ResumeLatest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionState {
    pub resume_strategy: ResumeStrategy,
    pub session_key: Option<String>,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub agent: String,
    pub phase: String,
    pub timestamp: String,
    pub session_key: Option<String>,
    pub resume_strategy: ResumeStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub phase: Phase,
    pub iteration: u32,
    pub max_iterations: u32,
    pub feature_name: String,
    pub objective: String,
    pub plan_file: PathBuf,
    pub feedback_file: PathBuf,
    pub last_feedback_status: Option<FeedbackStatus>,

    #[serde(default)]
    pub approval_overridden: bool,

    #[serde(default)]
    pub workflow_session_id: String,

    #[serde(default)]
    pub agent_sessions: HashMap<String, AgentSessionState>,

    #[serde(default)]
    pub invocations: Vec<InvocationRecord>,
}

impl State {
    pub fn new(feature_name: &str, objective: &str, max_iterations: u32) -> Self {
        let sanitized_name = feature_name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>();

        let prefix = generate_unique_prefix();
        let plan_file = generate_plan_filename(&prefix, &sanitized_name);
        let feedback_file = generate_feedback_filename(&prefix, &sanitized_name, 1);

        Self {
            phase: Phase::Planning,
            iteration: 1,
            max_iterations,
            feature_name: feature_name.to_string(),
            objective: objective.to_string(),
            plan_file,
            feedback_file,
            last_feedback_status: None,
            approval_overridden: false,
            workflow_session_id: Uuid::new_v4().to_string(),
            agent_sessions: HashMap::new(),
            invocations: Vec::new(),
        }
    }

    /// Updates the feedback filename for a new iteration/round.
    /// This should be called before each review phase to generate a new feedback filename.
    pub fn update_feedback_for_iteration(&mut self, iteration: u32) {
        // Try to extract the unique prefix from the plan file
        let prefix = extract_unique_prefix(&self.plan_file)
            .unwrap_or_else(generate_unique_prefix);

        // Extract the sanitized name from the plan file
        let sanitized_name = extract_sanitized_name(&self.plan_file)
            .unwrap_or_else(|| {
                // Fallback: sanitize feature_name
                self.feature_name
                    .to_lowercase()
                    .replace(' ', "-")
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-')
                    .collect::<String>()
            });

        self.feedback_file = generate_feedback_filename(&prefix, &sanitized_name, iteration);
    }

    pub fn get_or_create_agent_session(
        &mut self,
        agent: &str,
        strategy: ResumeStrategy,
    ) -> &AgentSessionState {
        let now = chrono::Utc::now().to_rfc3339();

        if !self.agent_sessions.contains_key(agent) {
            let session_key = match strategy {
                ResumeStrategy::SessionId => Some(Uuid::new_v4().to_string()),
                _ => None,
            };

            self.agent_sessions.insert(
                agent.to_string(),
                AgentSessionState {
                    resume_strategy: strategy,
                    session_key,
                    last_used_at: now.clone(),
                },
            );
        }

        let session = self.agent_sessions.get_mut(agent).unwrap();
        session.last_used_at = now;
        session
    }

    pub fn record_invocation(&mut self, agent: &str, phase: &str) {
        let session = self.agent_sessions.get(agent);
        let (session_key, resume_strategy) = session
            .map(|s| (s.session_key.clone(), s.resume_strategy.clone()))
            .unwrap_or((None, ResumeStrategy::Stateless));

        self.invocations.push(InvocationRecord {
            agent: agent.to_string(),
            phase: phase.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            session_key,
            resume_strategy,
        });
    }

    pub fn ensure_workflow_session_id(&mut self) {
        if self.workflow_session_id.is_empty() {
            self.workflow_session_id = Uuid::new_v4().to_string();
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read state file: {}", path.display()))?;
        let mut state: State = serde_json::from_str(&content)
            .with_context(|| "Failed to parse state file as JSON")?;
        state.ensure_workflow_session_id();
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        // Extract working_dir from path: .planning-agent/<feature>.json -> working_dir
        // path.parent() gives .planning-agent, and parent of that gives working_dir
        if let Some(planning_dir) = path.parent() {
            if let Some(working_dir) = planning_dir.parent() {
                ensure_planning_agent_dir(working_dir)
                    .with_context(|| format!("Failed to create planning directory in: {}", working_dir.display()))?;
            } else {
                // Fallback: just create the parent directory
                fs::create_dir_all(planning_dir)
                    .with_context(|| format!("Failed to create directory: {}", planning_dir.display()))?;
            }
        }
        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize state to JSON")?;
        fs::write(path, content)
            .with_context(|| format!("Failed to write state file: {}", path.display()))?;
        Ok(())
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        // Extract working_dir from path: .planning-agent/<feature>.json -> working_dir
        // path.parent() gives .planning-agent, and parent of that gives working_dir
        if let Some(planning_dir) = path.parent() {
            if let Some(working_dir) = planning_dir.parent() {
                ensure_planning_agent_dir(working_dir)
                    .with_context(|| format!("Failed to create planning directory in: {}", working_dir.display()))?;
            } else {
                // Fallback: just create the parent directory
                fs::create_dir_all(planning_dir)
                    .with_context(|| format!("Failed to create directory: {}", planning_dir.display()))?;
            }
        }
        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize state to JSON")?;

        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, &content)
            .with_context(|| format!("Failed to write temp state file: {}", temp_path.display()))?;
        fs::rename(&temp_path, path)
            .with_context(|| format!("Failed to rename temp file to: {}", path.display()))?;
        Ok(())
    }

    pub fn transition(&mut self, to: Phase) -> Result<()> {
        let valid = matches!(
            (&self.phase, &to),
            (Phase::Planning, Phase::Reviewing)
                | (Phase::Reviewing, Phase::Revising)
                | (Phase::Reviewing, Phase::Complete)
                | (Phase::Revising, Phase::Reviewing)
        );

        if valid {
            self.phase = to;
            Ok(())
        } else {
            anyhow::bail!(
                "Invalid state transition from {:?} to {:?}",
                self.phase,
                to
            )
        }
    }

    pub fn should_continue(&self) -> bool {
        if self.phase == Phase::Complete {
            return false;
        }
        self.iteration <= self.max_iterations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state() {
        let state = State::new("user-auth", "Implement authentication", 3);
        assert_eq!(state.phase, Phase::Planning);
        assert_eq!(state.iteration, 1);

        // Plan file should have timestamp prefix: YYYYMMDD-HHMMSS-xxxxxxxx_user-auth.md
        let plan_file_str = state.plan_file.to_string_lossy();
        assert!(plan_file_str.starts_with("docs/plans/"));
        assert!(plan_file_str.ends_with("_user-auth.md"));
        // Verify timestamp-uuid prefix format (at least 24 chars before underscore)
        let filename = state.plan_file.file_name().unwrap().to_string_lossy();
        let underscore_pos = filename.find('_').expect("should have underscore");
        assert!(underscore_pos >= 24, "prefix should be at least 24 chars (got {})", underscore_pos);
    }

    #[test]
    fn test_new_state_feedback_file_has_round_number() {
        let state = State::new("user-auth", "Implement authentication", 3);

        // Feedback file should have timestamp prefix and round number: ..._feedback_1.md
        let feedback_file_str = state.feedback_file.to_string_lossy();
        assert!(feedback_file_str.starts_with("docs/plans/"));
        assert!(feedback_file_str.ends_with("_user-auth_feedback_1.md"));
    }

    #[test]
    fn test_update_feedback_for_iteration() {
        let mut state = State::new("test-feature", "Test objective", 3);

        // Initial feedback file should have round 1
        assert!(state.feedback_file.to_string_lossy().ends_with("_feedback_1.md"));

        // Update to round 2
        state.update_feedback_for_iteration(2);
        assert!(state.feedback_file.to_string_lossy().ends_with("_feedback_2.md"));

        // Update to round 3
        state.update_feedback_for_iteration(3);
        assert!(state.feedback_file.to_string_lossy().ends_with("_feedback_3.md"));
    }

    #[test]
    fn test_extract_unique_prefix_new_format() {
        let plan_file = PathBuf::from("docs/plans/20250101-120000-abcd1234_my-feature.md");
        let prefix = extract_unique_prefix(&plan_file);
        assert_eq!(prefix, Some("20250101-120000-abcd1234".to_string()));
    }

    #[test]
    fn test_extract_unique_prefix_legacy_format() {
        let plan_file = PathBuf::from("docs/plans/existing-feature.md");
        let prefix = extract_unique_prefix(&plan_file);
        assert_eq!(prefix, None);
    }

    #[test]
    fn test_extract_sanitized_name_new_format() {
        let plan_file = PathBuf::from("docs/plans/20250101-120000-abcd1234_my-feature.md");
        let name = extract_sanitized_name(&plan_file);
        assert_eq!(name, Some("my-feature".to_string()));
    }

    #[test]
    fn test_extract_sanitized_name_legacy_format() {
        let plan_file = PathBuf::from("docs/plans/existing-feature.md");
        let name = extract_sanitized_name(&plan_file);
        assert_eq!(name, Some("existing-feature".to_string()));
    }

    #[test]
    fn test_update_feedback_for_iteration_with_legacy_plan_file() {
        // Simulate loading a state with legacy plan file format
        let mut state = State::new("test", "test", 3);
        // Manually set to legacy format
        state.plan_file = PathBuf::from("docs/plans/existing-feature.md");
        state.feedback_file = PathBuf::from("docs/plans/existing-feature_feedback.md");

        // Update to round 2 - should generate a new prefix
        state.update_feedback_for_iteration(2);

        // Feedback file should have a new timestamp prefix and round number
        let feedback_str = state.feedback_file.to_string_lossy();
        assert!(feedback_str.contains("_existing-feature_feedback_2.md"), "got: {}", feedback_str);
    }

    #[test]
    fn test_valid_transitions() {
        let mut state = State::new("test", "test", 3);

        assert!(state.transition(Phase::Reviewing).is_ok());
        assert_eq!(state.phase, Phase::Reviewing);

        assert!(state.transition(Phase::Revising).is_ok());
        assert_eq!(state.phase, Phase::Revising);

        assert!(state.transition(Phase::Reviewing).is_ok());
        assert!(state.transition(Phase::Complete).is_ok());
    }

    #[test]
    fn test_invalid_transition() {
        let mut state = State::new("test", "test", 3);
        assert!(state.transition(Phase::Complete).is_err());
    }

    #[test]
    fn test_should_continue() {
        let mut state = State::new("test", "test", 2);
        assert!(state.should_continue());

        state.iteration = 3;
        assert!(!state.should_continue());

        state.iteration = 1;
        state.phase = Phase::Complete;
        assert!(!state.should_continue());
    }

    #[test]
    fn test_new_state_has_workflow_session_id() {
        let state = State::new("test", "test objective", 3);
        assert!(!state.workflow_session_id.is_empty());
        assert!(state.agent_sessions.is_empty());
        assert!(state.invocations.is_empty());
    }

    #[test]
    fn test_workflow_session_id_is_stable() {
        let state = State::new("test", "test objective", 3);
        let session_id = state.workflow_session_id.clone();
        assert_eq!(state.workflow_session_id, session_id);
    }

    #[test]
    fn test_get_or_create_agent_session_stateless() {
        let mut state = State::new("test", "test objective", 3);
        let session = state.get_or_create_agent_session("claude", ResumeStrategy::Stateless);

        assert_eq!(session.resume_strategy, ResumeStrategy::Stateless);
        assert!(session.session_key.is_none());
        assert!(!session.last_used_at.is_empty());
    }

    #[test]
    fn test_get_or_create_agent_session_with_session_id() {
        let mut state = State::new("test", "test objective", 3);
        let session = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);

        assert_eq!(session.resume_strategy, ResumeStrategy::SessionId);
        assert!(session.session_key.is_some());
        let session_key = session.session_key.clone().unwrap();
        assert!(!session_key.is_empty());
    }

    #[test]
    fn test_agent_session_is_reused() {
        let mut state = State::new("test", "test objective", 3);

        let session1 = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        let key1 = session1.session_key.clone();

        let session2 = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        let key2 = session2.session_key.clone();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_record_invocation() {
        let mut state = State::new("test", "test objective", 3);
        state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        state.record_invocation("claude", "Planning");

        assert_eq!(state.invocations.len(), 1);
        let inv = &state.invocations[0];
        assert_eq!(inv.agent, "claude");
        assert_eq!(inv.phase, "Planning");
        assert!(!inv.timestamp.is_empty());
        assert!(inv.session_key.is_some());
        assert_eq!(inv.resume_strategy, ResumeStrategy::SessionId);
    }

    #[test]
    fn test_ensure_workflow_session_id() {
        let mut state = State::new("test", "test objective", 3);
        state.workflow_session_id = String::new();
        assert!(state.workflow_session_id.is_empty());

        state.ensure_workflow_session_id();
        assert!(!state.workflow_session_id.is_empty());
    }

    #[test]
    fn test_backward_compatibility_with_existing_state() {
        let old_state_json = r#"{
            "phase": "reviewing",
            "iteration": 2,
            "max_iterations": 3,
            "feature_name": "existing-feature",
            "objective": "Some objective",
            "plan_file": "docs/plans/existing-feature.md",
            "feedback_file": "docs/plans/existing-feature_feedback.md",
            "last_feedback_status": "needs_revision",
            "approval_overridden": false
        }"#;

        let state: State = serde_json::from_str(old_state_json).unwrap();
        assert_eq!(state.feature_name, "existing-feature");
        assert!(state.workflow_session_id.is_empty());
        assert!(state.agent_sessions.is_empty());
        assert!(state.invocations.is_empty());
    }

    #[test]
    fn test_state_serialization_with_session_data() {
        let mut state = State::new("test", "test objective", 3);
        state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        state.record_invocation("claude", "Planning");

        let json = serde_json::to_string(&state).unwrap();
        let loaded: State = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.workflow_session_id, state.workflow_session_id);
        assert_eq!(loaded.agent_sessions.len(), 1);
        assert!(loaded.agent_sessions.contains_key("claude"));
        assert_eq!(loaded.invocations.len(), 1);
    }

    #[test]
    fn test_phase_label_short() {
        assert_eq!(PhaseLabel::Planning.short(), "Plan");
        assert_eq!(PhaseLabel::Reviewing.short(), "Review");
        assert_eq!(PhaseLabel::Revising.short(), "Revise");
        assert_eq!(PhaseLabel::Complete.short(), "Done");
    }

    #[test]
    fn test_phase_label_full() {
        assert_eq!(PhaseLabel::Planning.full(), "Planning");
        assert_eq!(PhaseLabel::Reviewing.full(), "Reviewing");
        assert_eq!(PhaseLabel::Revising.full(), "Revising");
        assert_eq!(PhaseLabel::Complete.full(), "Complete");
    }

    #[test]
    fn test_phase_label_with_iteration() {
        assert_eq!(PhaseLabel::Planning.with_iteration(1), "Planning");
        assert_eq!(PhaseLabel::Reviewing.with_iteration(1), "Reviewing");
        assert_eq!(PhaseLabel::Reviewing.with_iteration(2), "Reviewing #2");
        assert_eq!(PhaseLabel::Revising.with_iteration(1), "Revising #1");
        assert_eq!(PhaseLabel::Revising.with_iteration(3), "Revising #3");
        assert_eq!(PhaseLabel::Complete.with_iteration(5), "Complete");
    }

    #[test]
    fn test_phase_label_display() {
        assert_eq!(format!("{}", PhaseLabel::Planning), "Planning");
        assert_eq!(format!("{}", PhaseLabel::Reviewing), "Reviewing");
    }

    #[test]
    fn test_phase_to_label() {
        assert_eq!(Phase::Planning.label(), PhaseLabel::Planning);
        assert_eq!(Phase::Reviewing.label(), PhaseLabel::Reviewing);
        assert_eq!(Phase::Revising.label(), PhaseLabel::Revising);
        assert_eq!(Phase::Complete.label(), PhaseLabel::Complete);
    }
}
