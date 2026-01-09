use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Phase of the verification workflow.
/// Unlike the planning `Phase` enum, this is for the post-implementation verification loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationPhase {
    Verifying,
    Fixing,
    Complete,
}

/// State for the verification workflow.
/// This is separate from the planning `State` struct and tracks the verification/fixing loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationState {
    /// Path to plan folder (normalized, not the plan.md file)
    pub plan_path: PathBuf,
    /// Repository directory to verify against
    pub working_dir: PathBuf,
    /// Current phase in the verification workflow
    pub phase: VerificationPhase,
    /// Current verification iteration (starts at 1)
    pub iteration: u32,
    /// Maximum allowed iterations before stopping
    pub max_iterations: u32,
    /// Last verification verdict: "APPROVED" or "NEEDS_REVISION"
    pub last_verdict: Option<String>,
    /// Session ID for telemetry correlation
    pub workflow_session_id: String,
    /// Timestamp of last state update (RFC3339 format)
    #[serde(default)]
    pub updated_at: String,
}

impl VerificationState {
    /// Creates a new VerificationState for a plan.
    pub fn new(
        plan_path: PathBuf,
        working_dir: PathBuf,
        max_iterations: u32,
        session_id: Option<String>,
    ) -> Self {
        Self {
            plan_path,
            working_dir,
            phase: VerificationPhase::Verifying,
            iteration: 1,
            max_iterations,
            last_verdict: None,
            workflow_session_id: session_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Returns the path to the verification state file within the plan folder.
    pub fn state_file_path(plan_path: &Path) -> PathBuf {
        plan_path.join("verification_state.json")
    }

    /// Returns the path for a verification report file.
    /// Format: plan_path/verification_<iteration>.md
    pub fn verification_report_path(&self) -> PathBuf {
        self.plan_path
            .join(format!("verification_{}.md", self.iteration))
    }

    /// Returns the path to the plan.md file.
    pub fn plan_file_path(&self) -> PathBuf {
        self.plan_path.join("plan.md")
    }

    /// Loads existing VerificationState from a plan folder if it exists.
    pub fn load(plan_path: &Path) -> Result<Option<Self>> {
        let state_file = Self::state_file_path(plan_path);
        if !state_file.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&state_file)
            .with_context(|| format!("Failed to read verification state: {}", state_file.display()))?;
        let state: Self = serde_json::from_str(&content)
            .with_context(|| "Failed to parse verification state")?;
        Ok(Some(state))
    }

    /// Saves the verification state atomically using write-then-rename pattern.
    pub fn save(&self) -> Result<()> {
        let state_file = Self::state_file_path(&self.plan_path);
        let temp_file = state_file.with_extension("json.tmp");

        // Ensure parent directory exists
        if let Some(parent) = state_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(self)
            .with_context(|| "Failed to serialize verification state")?;
        fs::write(&temp_file, &content)
            .with_context(|| format!("Failed to write temp file: {}", temp_file.display()))?;
        fs::rename(&temp_file, &state_file)
            .with_context(|| format!("Failed to rename to: {}", state_file.display()))?;
        Ok(())
    }

    /// Returns whether the verification workflow should continue.
    /// Returns false when phase is Complete or max iterations exceeded.
    pub fn should_continue(&self) -> bool {
        match self.phase {
            VerificationPhase::Complete => false,
            _ => self.iteration <= self.max_iterations,
        }
    }

    /// Transitions to a new phase, validating the transition is allowed.
    /// Valid transitions:
    /// - Verifying -> Fixing (verification failed)
    /// - Verifying -> Complete (verification passed)
    /// - Fixing -> Verifying (fix applied, re-verify)
    pub fn transition(&mut self, to: VerificationPhase) -> Result<()> {
        let valid = matches!(
            (&self.phase, &to),
            (VerificationPhase::Verifying, VerificationPhase::Fixing)
                | (VerificationPhase::Verifying, VerificationPhase::Complete)
                | (VerificationPhase::Fixing, VerificationPhase::Verifying)
        );

        if valid {
            // Increment iteration when transitioning from Fixing -> Verifying
            if self.phase == VerificationPhase::Fixing && to == VerificationPhase::Verifying {
                self.iteration += 1;
            }
            self.phase = to;
            self.updated_at = chrono::Utc::now().to_rfc3339();
            Ok(())
        } else {
            anyhow::bail!(
                "Invalid verification transition from {:?} to {:?}",
                self.phase,
                to
            )
        }
    }

}

/// Normalizes a plan path to always point to the plan folder (not plan.md file).
/// Accepts either:
/// - A plan folder path (e.g., ~/.planning-agent/plans/20251230-123632-abc123_my-feature/)
/// - A plan file path (e.g., ~/.planning-agent/plans/20251230-123632-abc123_my-feature/plan.md)
///
/// Returns the plan folder path in both cases.
pub fn normalize_plan_path(path: &Path) -> PathBuf {
    if path.ends_with("plan.md") {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_new_verification_state() {
        let plan_path = PathBuf::from("/tmp/test-plan");
        let working_dir = PathBuf::from("/tmp/working");
        let state = VerificationState::new(plan_path.clone(), working_dir.clone(), 3, None);

        assert_eq!(state.phase, VerificationPhase::Verifying);
        assert_eq!(state.iteration, 1);
        assert_eq!(state.max_iterations, 3);
        assert_eq!(state.plan_path, plan_path);
        assert_eq!(state.working_dir, working_dir);
        assert!(state.last_verdict.is_none());
        assert!(!state.workflow_session_id.is_empty());
    }

    #[test]
    fn test_should_continue() {
        let mut state =
            VerificationState::new(PathBuf::from("/tmp"), PathBuf::from("/tmp"), 3, None);

        // Should continue in Verifying phase
        assert!(state.should_continue());

        // Should continue in Fixing phase
        state.phase = VerificationPhase::Fixing;
        assert!(state.should_continue());

        // Should not continue when Complete
        state.phase = VerificationPhase::Complete;
        assert!(!state.should_continue());

        // Should not continue when max iterations exceeded
        state.phase = VerificationPhase::Verifying;
        state.iteration = 4;
        assert!(!state.should_continue());
    }

    #[test]
    fn test_valid_transitions() {
        let mut state =
            VerificationState::new(PathBuf::from("/tmp"), PathBuf::from("/tmp"), 3, None);

        // Verifying -> Fixing
        assert!(state.transition(VerificationPhase::Fixing).is_ok());
        assert_eq!(state.phase, VerificationPhase::Fixing);
        assert_eq!(state.iteration, 1); // No increment yet

        // Fixing -> Verifying (should increment iteration)
        assert!(state.transition(VerificationPhase::Verifying).is_ok());
        assert_eq!(state.phase, VerificationPhase::Verifying);
        assert_eq!(state.iteration, 2); // Incremented

        // Verifying -> Complete
        assert!(state.transition(VerificationPhase::Complete).is_ok());
        assert_eq!(state.phase, VerificationPhase::Complete);
    }

    #[test]
    fn test_invalid_transitions() {
        let mut state =
            VerificationState::new(PathBuf::from("/tmp"), PathBuf::from("/tmp"), 3, None);

        // Verifying -> Verifying (invalid)
        assert!(state.transition(VerificationPhase::Verifying).is_err());

        // Fixing -> Complete (invalid)
        state.phase = VerificationPhase::Fixing;
        assert!(state.transition(VerificationPhase::Complete).is_err());

        // Complete -> anything (invalid)
        state.phase = VerificationPhase::Complete;
        assert!(state.transition(VerificationPhase::Verifying).is_err());
        assert!(state.transition(VerificationPhase::Fixing).is_err());
    }

    #[test]
    fn test_normalize_plan_path_folder() {
        let path = PathBuf::from("/home/user/.planning-agent/plans/20251230-abc_feature");
        let normalized = normalize_plan_path(&path);
        assert_eq!(normalized, path);
    }

    #[test]
    fn test_normalize_plan_path_file() {
        let path = PathBuf::from("/home/user/.planning-agent/plans/20251230-abc_feature/plan.md");
        let normalized = normalize_plan_path(&path);
        assert_eq!(
            normalized,
            PathBuf::from("/home/user/.planning-agent/plans/20251230-abc_feature")
        );
    }

    #[test]
    fn test_verification_report_path() {
        let state = VerificationState::new(
            PathBuf::from("/tmp/plan-folder"),
            PathBuf::from("/tmp/working"),
            3,
            None,
        );

        assert_eq!(
            state.verification_report_path(),
            PathBuf::from("/tmp/plan-folder/verification_1.md")
        );
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let plan_path = temp_dir.path().to_path_buf();

        let mut state =
            VerificationState::new(plan_path.clone(), PathBuf::from("/tmp/working"), 3, None);
        state.last_verdict = Some("NEEDS_REVISION".to_string());

        // Save
        state.save().unwrap();

        // Load
        let loaded = VerificationState::load(&plan_path).unwrap().unwrap();
        assert_eq!(loaded.phase, VerificationPhase::Verifying);
        assert_eq!(loaded.iteration, 1);
        assert_eq!(loaded.max_iterations, 3);
        assert_eq!(loaded.last_verdict, Some("NEEDS_REVISION".to_string()));
    }

    #[test]
    fn test_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let result = VerificationState::load(temp_dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let state = VerificationState::new(
            PathBuf::from("/tmp/plan"),
            PathBuf::from("/tmp/working"),
            5,
            Some("test-session-id".to_string()),
        );

        let json = serde_json::to_string(&state).unwrap();
        let loaded: VerificationState = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.plan_path, state.plan_path);
        assert_eq!(loaded.working_dir, state.working_dir);
        assert_eq!(loaded.phase, state.phase);
        assert_eq!(loaded.iteration, state.iteration);
        assert_eq!(loaded.max_iterations, state.max_iterations);
        assert_eq!(loaded.workflow_session_id, state.workflow_session_id);
    }

}
