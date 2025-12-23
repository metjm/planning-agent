use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Reviewing,
    Revising,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Approved,
    NeedsRevision,
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
}

impl State {
    pub fn new(feature_name: &str, objective: &str, max_iterations: u32) -> Self {
        let sanitized_name = feature_name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>();

        Self {
            phase: Phase::Planning,
            iteration: 1,
            max_iterations,
            feature_name: feature_name.to_string(),
            objective: objective.to_string(),
            plan_file: PathBuf::from(format!("docs/plans/{}.md", sanitized_name)),
            feedback_file: PathBuf::from(format!("docs/plans/{}_feedback.md", sanitized_name)),
            last_feedback_status: None,
            approval_overridden: false,
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read state file: {}", path.display()))?;
        let state: State = serde_json::from_str(&content)
            .with_context(|| "Failed to parse state file as JSON")?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
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
        assert_eq!(state.plan_file, PathBuf::from("docs/plans/user-auth.md"));
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
}
