//! Workflow input types for starting and resuming workflows.
//!
//! These types replace the legacy `State` struct as input to the workflow runner.
//! They provide a clean separation between input parameters and derived state.

use crate::domain::types::{FeatureName, MaxIterations, Objective, WorkflowId, WorktreeState};

/// Input parameters for starting a new workflow.
#[derive(Debug, Clone)]
pub struct NewWorkflowInput {
    /// Human-readable feature name for display.
    pub feature_name: FeatureName,
    /// User objective describing what should be accomplished.
    pub objective: Objective,
    /// Maximum iterations before stopping.
    pub max_iterations: MaxIterations,
    /// Optional worktree information for git worktree workflows.
    pub worktree_info: Option<WorktreeState>,
}

impl NewWorkflowInput {
    /// Creates a new workflow input with the given parameters.
    pub fn new(
        feature_name: impl Into<FeatureName>,
        objective: impl Into<Objective>,
        max_iterations: u32,
    ) -> Self {
        Self {
            feature_name: feature_name.into(),
            objective: objective.into(),
            max_iterations: MaxIterations(max_iterations),
            worktree_info: None,
        }
    }

    /// Sets the worktree info for git worktree workflows.
    pub fn with_worktree(mut self, worktree_info: WorktreeState) -> Self {
        self.worktree_info = Some(worktree_info);
        self
    }
}

/// Input parameters for resuming an existing workflow.
#[derive(Debug, Clone)]
pub struct ResumeWorkflowInput {
    /// The workflow session ID to resume.
    pub workflow_id: WorkflowId,
}

impl ResumeWorkflowInput {
    /// Creates a resume input from a workflow session ID string.
    pub fn from_session_id(session_id: &str) -> Result<Self, uuid::Error> {
        Ok(Self {
            workflow_id: WorkflowId::from_string(session_id)?,
        })
    }
}

/// Unified workflow input - either a new workflow or resuming an existing one.
#[derive(Debug, Clone)]
pub enum WorkflowInput {
    /// Start a new workflow with the given parameters.
    New(NewWorkflowInput),
    /// Resume an existing workflow by session ID.
    Resume(ResumeWorkflowInput),
}

impl WorkflowInput {
    /// Creates input for a new workflow.
    pub fn new_workflow(
        feature_name: impl Into<FeatureName>,
        objective: impl Into<Objective>,
        max_iterations: u32,
    ) -> Self {
        Self::New(NewWorkflowInput::new(
            feature_name,
            objective,
            max_iterations,
        ))
    }

    /// Creates input for resuming a workflow.
    pub fn resume(session_id: &str) -> Result<Self, uuid::Error> {
        Ok(Self::Resume(ResumeWorkflowInput::from_session_id(
            session_id,
        )?))
    }

    /// Returns the workflow session ID if resuming, or generates a new one.
    pub fn workflow_session_id(&self) -> WorkflowId {
        match self {
            Self::New(_) => WorkflowId::new(),
            Self::Resume(r) => r.workflow_id.clone(),
        }
    }
}
