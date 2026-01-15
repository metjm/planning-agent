use crate::app::failure::{FailureContext, MAX_FAILURE_HISTORY};
use crate::planning_paths;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Generates a unique folder name for a plan using timestamp and UUID.
/// Format: YYYYMMDD-HHMMSS-xxxxxxxx_<sanitized_name>
///
/// **DEPRECATED**: Only used for legacy plan structure.
/// New sessions use session-centric paths directly.
fn generate_plan_folder_name(sanitized_name: &str) -> String {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let uuid_suffix = &Uuid::new_v4().to_string()[..8];
    format!("{}-{}_{}", timestamp, uuid_suffix, sanitized_name)
}

/// Generates the plan file path inside a plan folder.
/// Format: ~/.planning-agent/plans/<folder>/plan.md
///
/// **DEPRECATED**: Only used for legacy plan structure.
/// New sessions use `planning_paths::session_plan_path()`.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
#[allow(dead_code)]
fn generate_plan_path(folder_name: &str) -> Result<PathBuf> {
    Ok(planning_paths::plans_dir()?.join(folder_name).join("plan.md"))
}

/// Generates a feedback file path inside a plan folder.
/// Format: ~/.planning-agent/plans/<folder>/feedback_<round>.md
///
/// **DEPRECATED**: Only used for legacy plan structure.
/// New sessions use `planning_paths::session_feedback_path()`.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
fn generate_feedback_path(folder_name: &str, round: u32) -> Result<PathBuf> {
    Ok(planning_paths::plans_dir()?
        .join(folder_name)
        .join(format!("feedback_{}.md", round)))
}

/// Extracts the plan folder name from a plan file path.
/// Works with both new format (in ~/.planning-agent/plans/) and legacy format (docs/plans/).
///
/// **DEPRECATED**: Only used for legacy plan structure.
fn extract_plan_folder(plan_file: &Path) -> Option<String> {
    // New format: ~/.planning-agent/plans/<folder>/plan.md
    // The folder is the parent of the plan file
    if let Some(parent) = plan_file.parent() {
        if let Some(folder_name) = parent.file_name() {
            let folder_str = folder_name.to_str()?;
            // Validate it looks like a timestamp-uuid prefix folder
            if folder_str.len() >= 24 && folder_str.chars().nth(8) == Some('-') {
                return Some(folder_str.to_string());
            }
        }
    }
    None
}

/// Extracts the sanitized feature name from a plan folder or legacy filename.
///
/// **DEPRECATED**: Only used for legacy plan structure.
fn extract_sanitized_name(plan_file: &Path) -> Option<String> {
    // Try new format first: folder name like "YYYYMMDD-HHMMSS-xxxxxxxx_feature-name"
    if let Some(folder_name) = extract_plan_folder(plan_file) {
        if let Some(underscore_pos) = folder_name.find('_') {
            return Some(folder_name[underscore_pos + 1..].to_string());
        }
    }

    // Legacy format: docs/plans/feature-name.md
    let filename = plan_file.file_stem()?.to_str()?;

    // Check for old timestamp format in filename
    if let Some(underscore_pos) = filename.find('_') {
        let prefix = &filename[..underscore_pos];
        if prefix.len() >= 24 && prefix.chars().nth(8) == Some('-') {
            return Some(filename[underscore_pos + 1..].to_string());
        }
    }

    // Plain legacy format
    Some(filename.to_string())
}

/// Checks if a plan file path uses the session-centric structure.
/// Session-centric paths contain a UUID session_id in the parent directory.
fn is_session_centric_path(plan_file: &Path) -> bool {
    if let Some(parent) = plan_file.parent() {
        if let Some(folder_name) = parent.file_name() {
            let folder_str = folder_name.to_string_lossy();
            // UUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx (36 chars with hyphens)
            return folder_str.len() == 36
                && folder_str.chars().filter(|c| *c == '-').count() == 4;
        }
    }
    false
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

    /// Timestamp of last state update (RFC3339 format).
    /// Used for conflict detection between session snapshots and state files.
    #[serde(default)]
    pub updated_at: String,

    /// Current failure context if the workflow is in a failed state.
    /// Used for recovery prompts and resume-time failure handling.
    #[serde(default)]
    pub last_failure: Option<FailureContext>,

    /// History of failures encountered during this workflow.
    /// Limited to MAX_FAILURE_HISTORY entries to prevent unbounded growth.
    #[serde(default)]
    pub failure_history: Vec<FailureContext>,

    /// Git worktree information if session is using a worktree
    #[serde(default)]
    pub worktree_info: Option<WorktreeState>,
}

/// Persisted worktree state for session resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeState {
    /// Path to the worktree directory
    pub worktree_path: PathBuf,
    /// Branch name in the worktree
    pub branch_name: String,
    /// Original branch to merge into
    pub source_branch: Option<String>,
    /// Original working directory (repo root)
    pub original_dir: PathBuf,
}

impl State {
    /// Creates a new State for a workflow.
    ///
    /// Uses the new session-centric directory structure:
    /// - Plan file: `~/.planning-agent/sessions/<session-id>/plan.md`
    /// - Feedback file: `~/.planning-agent/sessions/<session-id>/feedback_<round>.md`
    ///
    /// # Errors
    /// Returns an error if the home directory cannot be determined for plan storage.
    pub fn new(feature_name: &str, objective: &str, max_iterations: u32) -> Result<Self> {
        // Generate session ID first - this is the primary key for the session
        let workflow_session_id = Uuid::new_v4().to_string();

        // Use session-centric paths
        let plan_file = planning_paths::session_plan_path(&workflow_session_id)?;
        let feedback_file = planning_paths::session_feedback_path(&workflow_session_id, 1)?;

        Ok(Self {
            phase: Phase::Planning,
            iteration: 1,
            max_iterations,
            feature_name: feature_name.to_string(),
            objective: objective.to_string(),
            plan_file,
            feedback_file,
            last_feedback_status: None,
            approval_overridden: false,
            workflow_session_id,
            agent_sessions: HashMap::new(),
            invocations: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
            last_failure: None,
            failure_history: Vec::new(),
            worktree_info: None,
        })
    }

    /// Updates the feedback filename for a new iteration/round.
    /// This should be called before each review phase to generate a new feedback filename.
    pub fn update_feedback_for_iteration(&mut self, iteration: u32) {
        // For session-centric paths, use the session ID directly
        if is_session_centric_path(&self.plan_file) || !self.workflow_session_id.is_empty() {
            if let Ok(path) = planning_paths::session_feedback_path(&self.workflow_session_id, iteration) {
                self.feedback_file = path;
                return;
            }
        }

        // Legacy path handling: try to extract the folder name from the plan file path
        if let Some(folder_name) = extract_plan_folder(&self.plan_file) {
            if let Ok(path) = generate_feedback_path(&folder_name, iteration) {
                self.feedback_file = path;
                return;
            }
        }

        // Legacy fallback: generate a new folder for feedback
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

        // Generate a new folder for legacy plans
        let folder_name = generate_plan_folder_name(&sanitized_name);
        if let Ok(path) = generate_feedback_path(&folder_name, iteration) {
            self.feedback_file = path;
        }
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

    /// Sets the updated_at timestamp to the current time.
    /// Call this before saving to ensure the timestamp reflects the save time.
    pub fn set_updated_at(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Sets the updated_at timestamp to a specific value.
    /// Used for unified timestamps during stop operations.
    pub fn set_updated_at_with(&mut self, timestamp: &str) {
        self.updated_at = timestamp.to_string();
    }

    /// Returns true if this state has an updated_at timestamp.
    /// Legacy state files without updated_at will return false.
    pub fn has_updated_at(&self) -> bool {
        !self.updated_at.is_empty()
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
        // Create parent directory if needed (works for both home-based and legacy paths)
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize state to JSON")?;
        fs::write(path, content)
            .with_context(|| format!("Failed to write state file: {}", path.display()))?;
        Ok(())
    }

    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        // Create parent directory if needed (works for both home-based and legacy paths)
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
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

    /// Sets the current failure context and adds it to history.
    /// Trims history if it exceeds MAX_FAILURE_HISTORY.
    #[allow(dead_code)]
    pub fn set_failure(&mut self, failure: FailureContext) {
        self.failure_history.push(failure.clone());
        // Trim history if it exceeds the limit
        if self.failure_history.len() > MAX_FAILURE_HISTORY {
            let excess = self.failure_history.len() - MAX_FAILURE_HISTORY;
            self.failure_history.drain(0..excess);
        }
        self.last_failure = Some(failure);
    }

    /// Clears the current failure context (called after successful recovery).
    /// The failure remains in history for auditing.
    #[allow(dead_code)]
    pub fn clear_failure(&mut self) {
        self.last_failure = None;
    }

    /// Returns true if there's an active failure requiring recovery.
    #[allow(dead_code)]
    pub fn has_failure(&self) -> bool {
        self.last_failure.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state() {
        let state = State::new("user-auth", "Implement authentication", 3).unwrap();
        assert_eq!(state.phase, Phase::Planning);
        assert_eq!(state.iteration, 1);

        // Plan file should be in session directory: ~/.planning-agent/sessions/<session-id>/plan.md
        let plan_file_str = state.plan_file.to_string_lossy();
        assert!(plan_file_str.contains(".planning-agent/sessions/"), "got: {}", plan_file_str);
        assert!(plan_file_str.ends_with("/plan.md"), "got: {}", plan_file_str);
        // Verify session ID is in the path
        assert!(plan_file_str.contains(&state.workflow_session_id), "got: {}", plan_file_str);
    }

    #[test]
    fn test_new_state_feedback_file_has_round_number() {
        let state = State::new("user-auth", "Implement authentication", 3).unwrap();

        // Feedback file should be in session directory: ~/.planning-agent/sessions/<session-id>/feedback_1.md
        let feedback_file_str = state.feedback_file.to_string_lossy();
        assert!(feedback_file_str.contains(".planning-agent/sessions/"), "got: {}", feedback_file_str);
        assert!(feedback_file_str.ends_with("/feedback_1.md"), "got: {}", feedback_file_str);
    }

    #[test]
    fn test_update_feedback_for_iteration() {
        let mut state = State::new("test-feature", "Test objective", 3).unwrap();

        // Initial feedback file should have round 1
        assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_1.md"));

        // Update to round 2
        state.update_feedback_for_iteration(2);
        assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_2.md"));

        // Update to round 3
        state.update_feedback_for_iteration(3);
        assert!(state.feedback_file.to_string_lossy().ends_with("/feedback_3.md"));
    }

    #[test]
    fn test_extract_plan_folder_new_format() {
        // New format: ~/.planning-agent/plans/YYYYMMDD-HHMMSS-xxxxxxxx_my-feature/plan.md
        let plan_file = PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
        let folder = extract_plan_folder(&plan_file);
        assert_eq!(folder, Some("20250101-120000-abcd1234_my-feature".to_string()));
    }

    #[test]
    fn test_extract_plan_folder_legacy_format() {
        let plan_file = PathBuf::from("docs/plans/existing-feature.md");
        let folder = extract_plan_folder(&plan_file);
        assert_eq!(folder, None);
    }

    #[test]
    fn test_extract_sanitized_name_new_format() {
        // New format: folder contains the feature name
        let plan_file = PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
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
    fn test_is_session_centric_path() {
        // Session-centric path (UUID in parent)
        let session_path = PathBuf::from("/home/user/.planning-agent/sessions/550e8400-e29b-41d4-a716-446655440000/plan.md");
        assert!(is_session_centric_path(&session_path));

        // Legacy plan path (timestamp-uuid_feature format)
        let legacy_path = PathBuf::from("/home/user/.planning-agent/plans/20250101-120000-abcd1234_my-feature/plan.md");
        assert!(!is_session_centric_path(&legacy_path));

        // Docs path
        let docs_path = PathBuf::from("docs/plans/feature.md");
        assert!(!is_session_centric_path(&docs_path));
    }

    #[test]
    fn test_update_feedback_for_iteration_with_legacy_plan_file() {
        // Simulate loading a state with legacy plan file format
        let mut state = State::new("test", "test", 3).unwrap();
        let session_id = state.workflow_session_id.clone();
        // Manually set to legacy format
        state.plan_file = PathBuf::from("docs/plans/existing-feature.md");
        state.feedback_file = PathBuf::from("docs/plans/existing-feature_feedback.md");

        // Update to round 2 - should use session-centric path since session_id is set
        state.update_feedback_for_iteration(2);

        // Feedback file should use session directory since workflow_session_id is present
        let feedback_str = state.feedback_file.to_string_lossy();
        assert!(feedback_str.contains(".planning-agent/sessions/"), "got: {}", feedback_str);
        assert!(feedback_str.ends_with("/feedback_2.md"), "got: {}", feedback_str);
        assert!(feedback_str.contains(&session_id), "got: {}", feedback_str);
    }

    #[test]
    fn test_update_feedback_for_iteration_with_legacy_plan_file_no_session_id() {
        // Simulate loading a very old state with no session_id
        let mut state = State::new("test", "test", 3).unwrap();
        // Clear session ID to simulate legacy state
        state.workflow_session_id = String::new();
        // Manually set to legacy format
        state.plan_file = PathBuf::from("docs/plans/existing-feature.md");
        state.feedback_file = PathBuf::from("docs/plans/existing-feature_feedback.md");

        // Update to round 2 - should generate a new folder for feedback (legacy path)
        state.update_feedback_for_iteration(2);

        // Feedback file should be in a new folder with the proper format
        let feedback_str = state.feedback_file.to_string_lossy();
        assert!(feedback_str.contains(".planning-agent/plans/"), "got: {}", feedback_str);
        assert!(feedback_str.ends_with("/feedback_2.md"), "got: {}", feedback_str);
        assert!(feedback_str.contains("_existing-feature/"), "got: {}", feedback_str);
    }

    #[test]
    fn test_valid_transitions() {
        let mut state = State::new("test", "test", 3).unwrap();

        assert!(state.transition(Phase::Reviewing).is_ok());
        assert_eq!(state.phase, Phase::Reviewing);

        assert!(state.transition(Phase::Revising).is_ok());
        assert_eq!(state.phase, Phase::Revising);

        assert!(state.transition(Phase::Reviewing).is_ok());
        assert!(state.transition(Phase::Complete).is_ok());
    }

    #[test]
    fn test_invalid_transition() {
        let mut state = State::new("test", "test", 3).unwrap();
        assert!(state.transition(Phase::Complete).is_err());
    }

    #[test]
    fn test_should_continue() {
        let mut state = State::new("test", "test", 2).unwrap();
        assert!(state.should_continue());

        state.iteration = 3;
        assert!(!state.should_continue());

        state.iteration = 1;
        state.phase = Phase::Complete;
        assert!(!state.should_continue());
    }

    #[test]
    fn test_new_state_has_workflow_session_id() {
        let state = State::new("test", "test objective", 3).unwrap();
        assert!(!state.workflow_session_id.is_empty());
        assert!(state.agent_sessions.is_empty());
        assert!(state.invocations.is_empty());
    }

    #[test]
    fn test_workflow_session_id_is_stable() {
        let state = State::new("test", "test objective", 3).unwrap();
        let session_id = state.workflow_session_id.clone();
        assert_eq!(state.workflow_session_id, session_id);
    }

    #[test]
    fn test_get_or_create_agent_session_stateless() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        let session = state.get_or_create_agent_session("claude", ResumeStrategy::Stateless);

        assert_eq!(session.resume_strategy, ResumeStrategy::Stateless);
        assert!(session.session_key.is_none());
        assert!(!session.last_used_at.is_empty());
    }

    #[test]
    fn test_get_or_create_agent_session_with_session_id() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        let session = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);

        assert_eq!(session.resume_strategy, ResumeStrategy::SessionId);
        assert!(session.session_key.is_some());
        let session_key = session.session_key.clone().unwrap();
        assert!(!session_key.is_empty());
    }

    #[test]
    fn test_agent_session_is_reused() {
        let mut state = State::new("test", "test objective", 3).unwrap();

        let session1 = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        let key1 = session1.session_key.clone();

        let session2 = state.get_or_create_agent_session("claude", ResumeStrategy::SessionId);
        let key2 = session2.session_key.clone();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_record_invocation() {
        let mut state = State::new("test", "test objective", 3).unwrap();
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
        let mut state = State::new("test", "test objective", 3).unwrap();
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
        let mut state = State::new("test", "test objective", 3).unwrap();
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

    #[test]
    fn test_new_state_has_updated_at() {
        let state = State::new("test", "test objective", 3).unwrap();
        assert!(!state.updated_at.is_empty());
        assert!(state.has_updated_at());
    }

    #[test]
    fn test_set_updated_at() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        let original = state.updated_at.clone();

        // Wait a tiny bit and update
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.set_updated_at();

        // Timestamp should have changed
        assert_ne!(state.updated_at, original);
        assert!(state.has_updated_at());
    }

    #[test]
    fn test_set_updated_at_with() {
        let mut state = State::new("test", "test objective", 3).unwrap();
        let custom_time = "2025-12-29T15:00:00Z";
        state.set_updated_at_with(custom_time);
        assert_eq!(state.updated_at, custom_time);
    }

    #[test]
    fn test_legacy_state_without_updated_at() {
        // Simulate loading a legacy state file without updated_at field
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
        // updated_at should default to empty string
        assert!(state.updated_at.is_empty());
        assert!(!state.has_updated_at());
    }

    #[test]
    fn test_set_failure() {
        use crate::app::failure::{FailureContext, FailureKind};

        let mut state = State::new("test", "test objective", 3).unwrap();
        assert!(!state.has_failure());
        assert!(state.last_failure.is_none());
        assert!(state.failure_history.is_empty());

        let failure = FailureContext::new(
            FailureKind::Network,
            Phase::Reviewing,
            Some("codex".to_string()),
            2,
        );
        state.set_failure(failure);

        assert!(state.has_failure());
        assert!(state.last_failure.is_some());
        assert_eq!(state.failure_history.len(), 1);

        let last = state.last_failure.as_ref().unwrap();
        assert_eq!(last.kind, FailureKind::Network);
        assert_eq!(last.phase, Phase::Reviewing);
        assert_eq!(last.agent_name, Some("codex".to_string()));
    }

    #[test]
    fn test_clear_failure() {
        use crate::app::failure::{FailureContext, FailureKind};

        let mut state = State::new("test", "test objective", 3).unwrap();
        let failure = FailureContext::new(FailureKind::Timeout, Phase::Planning, None, 2);
        state.set_failure(failure);

        assert!(state.has_failure());
        state.clear_failure();

        assert!(!state.has_failure());
        assert!(state.last_failure.is_none());
        // History should still have the failure
        assert_eq!(state.failure_history.len(), 1);
    }

    #[test]
    fn test_failure_history_trimming() {
        use crate::app::failure::{FailureContext, FailureKind, MAX_FAILURE_HISTORY};

        let mut state = State::new("test", "test objective", 3).unwrap();

        // Add more failures than the limit
        for i in 0..(MAX_FAILURE_HISTORY + 10) {
            let failure = FailureContext::new(
                FailureKind::Network,
                Phase::Reviewing,
                Some(format!("agent-{}", i)),
                2,
            );
            state.set_failure(failure);
        }

        // History should be trimmed to MAX_FAILURE_HISTORY
        assert_eq!(state.failure_history.len(), MAX_FAILURE_HISTORY);

        // The oldest failures should have been removed
        // The first remaining failure should be agent-10 (since we added 60 and kept 50)
        let first = &state.failure_history[0];
        assert_eq!(first.agent_name, Some("agent-10".to_string()));
    }

    #[test]
    fn test_state_serialization_with_failure() {
        use crate::app::failure::{FailureContext, FailureKind};

        let mut state = State::new("test", "test objective", 3).unwrap();
        let failure = FailureContext::new(
            FailureKind::AllReviewersFailed,
            Phase::Reviewing,
            None,
            3,
        );
        state.set_failure(failure);

        let json = serde_json::to_string(&state).unwrap();
        let loaded: State = serde_json::from_str(&json).unwrap();

        assert!(loaded.has_failure());
        assert_eq!(loaded.failure_history.len(), 1);
        let last = loaded.last_failure.as_ref().unwrap();
        assert_eq!(last.kind, FailureKind::AllReviewersFailed);
    }

    #[test]
    fn test_backward_compatibility_without_failure_fields() {
        // Simulate loading a state without failure fields (pre-failure-handling state)
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
        // Failure fields should default properly
        assert!(state.last_failure.is_none());
        assert!(state.failure_history.is_empty());
        assert!(!state.has_failure());
    }
}
