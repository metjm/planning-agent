use crate::app::failure::{FailureContext, MAX_FAILURE_HISTORY};
use crate::planning_paths;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Reviewing,
    Revising,
    Complete,
}

/// Sub-phases within the implementation workflow.
/// This is used by the implementation orchestrator to track progress.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationPhase {
    /// Initial phase: implementing the approved plan
    #[default]
    Implementing,
    /// Review phase: reviewing the implementation for completeness
    ImplementationReview,
    /// Implementation complete and approved
    Complete,
}

/// UI mode for theming and display purposes.
/// Determines which color palette and phase labels to use in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiMode {
    /// Planning workflow is active (planning, reviewing, revising phases)
    #[default]
    Planning,
    /// Implementation workflow is active (implementing, implementation review phases)
    Implementation,
}

#[allow(dead_code)]
impl ImplementationPhase {
    /// Returns a human-readable label for this phase.
    pub fn label(&self) -> &'static str {
        match self {
            ImplementationPhase::Implementing => "Implementing",
            ImplementationPhase::ImplementationReview => "Reviewing Implementation",
            ImplementationPhase::Complete => "Implementation Complete",
        }
    }
}

/// State for the implementation workflow phase.
///
/// This is persisted as part of the main State and used by the
/// implementation orchestrator to track implement/review iterations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImplementationPhaseState {
    /// Current sub-phase within implementation workflow
    pub phase: ImplementationPhase,
    /// Current iteration (1-indexed)
    pub iteration: u32,
    /// Maximum allowed iterations before giving up
    pub max_iterations: u32,
    /// Last verdict from implementation review (if any)
    pub last_verdict: Option<String>,
    /// Last feedback from implementation review (for use in re-implementation)
    pub last_feedback: Option<String>,
}

#[allow(dead_code)]
impl ImplementationPhaseState {
    /// Creates a new implementation phase state with the given max iterations.
    pub fn new(max_iterations: u32) -> Self {
        Self {
            phase: ImplementationPhase::Implementing,
            iteration: 1,
            max_iterations,
            last_verdict: None,
            last_feedback: None,
        }
    }

    /// Returns true if we can continue with another iteration.
    pub fn can_continue(&self) -> bool {
        self.phase != ImplementationPhase::Complete && self.iteration <= self.max_iterations
    }

    /// Returns true if the last verdict was APPROVED.
    pub fn is_approved(&self) -> bool {
        self.last_verdict
            .as_ref()
            .map(|v| v == "APPROVED")
            .unwrap_or(false)
    }

    /// Transitions to the next phase.
    pub fn advance_to_review(&mut self) {
        self.phase = ImplementationPhase::ImplementationReview;
    }

    /// Transitions back to implementing for another round.
    pub fn advance_to_next_iteration(&mut self) {
        self.iteration += 1;
        self.phase = ImplementationPhase::Implementing;
    }

    /// Marks implementation as complete.
    pub fn mark_complete(&mut self) {
        self.phase = ImplementationPhase::Complete;
    }
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
    ConversationResume,
    ResumeLatest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConversationState {
    pub resume_strategy: ResumeStrategy,
    pub conversation_id: Option<String>,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub agent: String,
    pub phase: String,
    pub timestamp: String,
    pub conversation_id: Option<String>,
    pub resume_strategy: ResumeStrategy,
}

/// Serializable version of ReviewResult for state persistence.
/// ReviewResult from phases::reviewing is not Serialize, so we store the essential fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableReviewResult {
    pub agent_name: String,
    pub needs_revision: bool,
    pub feedback: String,
    pub summary: String,
}

/// Sequential review state: tracks progress through reviewer queue
/// and ensures all reviewers approve the same plan version.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SequentialReviewState {
    /// Index of the current reviewer in the current cycle order (0-indexed)
    pub current_reviewer_index: usize,
    /// Plan version counter - incremented each time the plan is modified (during revision)
    /// All reviewers must approve the same version for final approval
    pub plan_version: u32,
    /// The plan version that each reviewer last approved (reviewer_display_id -> version)
    /// When a reviewer approves, we record which version they approved
    pub approvals: HashMap<String, u32>,
    /// Accumulated approved reviews for summary generation.
    /// Stores (reviewer_display_id, SerializableReviewResult) pairs.
    /// Cleared when plan version changes (after revision).
    #[serde(default)]
    pub accumulated_reviews: Vec<(String, SerializableReviewResult)>,
    /// Total number of review runs per reviewer (reviewer_display_id -> count).
    /// Used for round-robin selection: the reviewer with the lowest count runs first.
    /// Persists across revisions and session resumes for balanced usage over time.
    #[serde(default)]
    pub reviewer_run_counts: HashMap<String, u32>,
    /// The reviewer order for the current cycle (computed at cycle start).
    /// Stored as display_ids in execution order. Cleared when cycle completes
    /// or is reset, so it's recomputed on the next cycle.
    #[serde(default)]
    pub current_cycle_order: Vec<String>,
    /// The reviewer who rejected the previous plan version.
    /// Used as a tiebreaker in round-robin selection: when reviewers have equal
    /// run counts, the previous rejecting reviewer goes first (to verify fix).
    /// Set when a reviewer rejects, cleared when consumed by start_new_cycle.
    #[serde(default)]
    pub last_rejecting_reviewer: Option<String>,
}

impl SequentialReviewState {
    /// Creates a new sequential review state for a fresh review cycle.
    pub fn new() -> Self {
        Self {
            current_reviewer_index: 0,
            plan_version: 1,
            approvals: HashMap::new(),
            accumulated_reviews: Vec::new(),
            reviewer_run_counts: HashMap::new(),
            current_cycle_order: Vec::new(),
            last_rejecting_reviewer: None,
        }
    }

    /// Called when a reviewer approves - records their approval and stores the review.
    pub fn record_approval(&mut self, reviewer_id: &str, review: &SerializableReviewResult) {
        self.approvals
            .insert(reviewer_id.to_string(), self.plan_version);
        // Remove any existing review from this reviewer (shouldn't happen but be safe)
        self.accumulated_reviews.retain(|(id, _)| id != reviewer_id);
        self.accumulated_reviews
            .push((reviewer_id.to_string(), review.clone()));
    }

    /// Called after revision - increments version and clears stale approvals and accumulated reviews.
    pub fn increment_version(&mut self) {
        self.plan_version += 1;
        // Clear all approvals and accumulated reviews - they're now stale since plan changed
        self.approvals.clear();
        self.accumulated_reviews.clear();
    }

    /// Checks if all reviewers have approved the current plan version.
    /// Takes &[&str] (reviewer display IDs) to avoid circular dependency with config.rs.
    pub fn all_approved(&self, reviewer_ids: &[&str]) -> bool {
        reviewer_ids
            .iter()
            .all(|id| self.approvals.get(*id) == Some(&self.plan_version))
    }

    /// Resets to first reviewer for a new cycle (after revision or config change).
    /// Also clears the cycle order so it's recomputed on next cycle.
    pub fn reset_to_first_reviewer(&mut self) {
        self.current_reviewer_index = 0;
        self.current_cycle_order.clear();
    }

    /// Advances to next reviewer.
    pub fn advance_to_next_reviewer(&mut self) {
        self.current_reviewer_index += 1;
    }

    /// Validates sequential review state against actual reviewer configuration.
    /// Checks both:
    /// 1. Index bounds: current_reviewer_index < number of reviewers (or cycle order length)
    /// 2. Cycle order validity: all entries in current_cycle_order exist in current config
    ///
    /// If either check fails, resets index to 0 and clears cycle order.
    /// Returns true if reset was needed (indicating config changed).
    ///
    /// Takes &[&str] (reviewer display IDs) to avoid circular dependency with config.rs.
    pub fn validate_reviewer_state(&mut self, reviewer_ids: &[&str]) -> bool {
        use std::collections::HashSet;
        let valid_ids: HashSet<&str> = reviewer_ids.iter().copied().collect();

        // Check if any entry in current_cycle_order is no longer in config
        let cycle_invalid = !self.current_cycle_order.is_empty()
            && self
                .current_cycle_order
                .iter()
                .any(|id| !valid_ids.contains(id.as_str()));

        // Check if index is out of bounds for the cycle order (if populated) or reviewer count
        let index_invalid = if self.current_cycle_order.is_empty() {
            self.current_reviewer_index >= reviewer_ids.len()
        } else {
            self.current_reviewer_index >= self.current_cycle_order.len()
        };

        if cycle_invalid || index_invalid {
            self.current_reviewer_index = 0;
            self.current_cycle_order.clear();
            true
        } else {
            false
        }
    }

    /// Increments the run count for a reviewer. Called before each review execution.
    pub fn increment_run_count(&mut self, reviewer_id: &str) {
        *self
            .reviewer_run_counts
            .entry(reviewer_id.to_string())
            .or_insert(0) += 1;
    }

    /// Returns the run count for a reviewer (0 if never run).
    pub fn get_run_count(&self, reviewer_id: &str) -> u32 {
        self.reviewer_run_counts
            .get(reviewer_id)
            .copied()
            .unwrap_or(0)
    }

    /// Records which reviewer rejected the plan.
    /// This is used as a tiebreaker in round-robin selection.
    pub fn record_rejection(&mut self, reviewer_id: &str) {
        self.last_rejecting_reviewer = Some(reviewer_id.to_string());
    }

    /// Starts a new review cycle by computing and storing the sorted reviewer order.
    /// Reviewers are sorted by:
    /// 1. Run count (ascending) - reviewer with lowest count runs first
    /// 2. Previous rejection (tiebreaker) - if tied, prefer the previous rejecting reviewer
    /// 3. Config order (stable sort) - if still tied, preserve config order
    ///
    /// Returns the ID of the previous rejecting reviewer if the tiebreaker was used,
    /// allowing callers to log when the tiebreaker affects ordering.
    ///
    /// Must be called when starting a new cycle.
    pub fn start_new_cycle(&mut self, reviewer_ids: &[&str]) -> Option<String> {
        let mut sorted: Vec<String> = reviewer_ids.iter().map(|s| (*s).to_string()).collect();

        // Capture the last rejecting reviewer for the closure and return value
        let last_rejector = self.last_rejecting_reviewer.take(); // take() also clears the field
        let tiebreaker_used = last_rejector.clone();

        sorted.sort_by(|a, b| {
            let count_a = self.get_run_count(a);
            let count_b = self.get_run_count(b);

            match count_a.cmp(&count_b) {
                std::cmp::Ordering::Equal => {
                    // Tiebreaker: prefer the previous rejecting reviewer
                    match (&last_rejector, a, b) {
                        (Some(rejector), a, _) if a == rejector => std::cmp::Ordering::Less,
                        (Some(rejector), _, b) if b == rejector => std::cmp::Ordering::Greater,
                        _ => std::cmp::Ordering::Equal, // Stable sort preserves config order
                    }
                }
                other => other,
            }
        });

        self.current_cycle_order = sorted;
        self.current_reviewer_index = 0;

        tiebreaker_used
    }

    /// Gets the current reviewer's display_id from the stored cycle order.
    /// Returns None if cycle order is empty (cycle not started).
    pub fn get_current_reviewer(&self) -> Option<&str> {
        self.current_cycle_order
            .get(self.current_reviewer_index)
            .map(|s| s.as_str())
    }

    /// Returns true if the cycle order needs to be (re)computed.
    /// This happens at the start of a new cycle or on session resume with empty order.
    pub fn needs_cycle_start(&self) -> bool {
        self.current_cycle_order.is_empty()
    }

    /// Converts accumulated SerializableReviewResults back to ReviewResults for summary generation.
    pub fn get_accumulated_reviews_for_summary(&self) -> Vec<crate::phases::ReviewResult> {
        self.accumulated_reviews
            .iter()
            .map(|(_, sr)| crate::phases::ReviewResult {
                agent_name: sr.agent_name.clone(),
                needs_revision: sr.needs_revision,
                feedback: sr.feedback.clone(),
                summary: sr.summary.clone(),
            })
            .collect()
    }
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
    pub agent_conversations: HashMap<String, AgentConversationState>,

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

    /// Implementation phase state for JSON-mode implementation workflow.
    /// Only present when implementation workflow is active.
    #[serde(default)]
    pub implementation_state: Option<ImplementationPhaseState>,

    /// Sequential review tracking state.
    /// Present when sequential review mode is active.
    #[serde(default)]
    pub sequential_review: Option<SequentialReviewState>,
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
            agent_conversations: HashMap::new(),
            invocations: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
            last_failure: None,
            failure_history: Vec::new(),
            worktree_info: None,
            implementation_state: None,
            sequential_review: None,
        })
    }

    /// Updates the feedback filename for a new iteration/round.
    /// This should be called before each review phase to generate a new feedback filename.
    pub fn update_feedback_for_iteration(&mut self, iteration: u32) {
        // Use session-centric paths with the workflow session ID
        if let Ok(path) =
            planning_paths::session_feedback_path(&self.workflow_session_id, iteration)
        {
            self.feedback_file = path;
        }
    }

    pub fn get_or_create_agent_session(
        &mut self,
        agent: &str,
        strategy: ResumeStrategy,
    ) -> &AgentConversationState {
        let now = chrono::Utc::now().to_rfc3339();

        if !self.agent_conversations.contains_key(agent) {
            // Don't pre-generate a conversation ID - it will be captured from the agent's output
            // after the first successful execution
            self.agent_conversations.insert(
                agent.to_string(),
                AgentConversationState {
                    resume_strategy: strategy,
                    conversation_id: None,
                    last_used_at: now.clone(),
                },
            );
        }

        let session = self.agent_conversations.get_mut(agent).unwrap();
        session.last_used_at = now;
        session
    }

    /// Update the conversation ID for an agent after capturing it from agent output.
    /// This is called after the agent runs and we capture the conversation ID from its init message.
    pub fn update_agent_conversation_id(&mut self, agent: &str, conversation_id: String) {
        if let Some(session) = self.agent_conversations.get_mut(agent) {
            session.conversation_id = Some(conversation_id);
            session.last_used_at = chrono::Utc::now().to_rfc3339();
        }
    }

    pub fn record_invocation(&mut self, agent: &str, phase: &str) {
        let session = self.agent_conversations.get(agent);
        let (conversation_id, resume_strategy) = session
            .map(|s| (s.conversation_id.clone(), s.resume_strategy.clone()))
            .unwrap_or((None, ResumeStrategy::Stateless));

        self.invocations.push(InvocationRecord {
            agent: agent.to_string(),
            phase: phase.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation_id,
            resume_strategy,
        });
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

    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read state file: {}", path.display()))?;
        let state: State =
            serde_json::from_str(&content).with_context(|| "Failed to parse state file as JSON")?;
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
            anyhow::bail!("Invalid state transition from {:?} to {:?}", self.phase, to)
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

    /// Returns the current UI mode based on implementation state.
    ///
    /// Returns `UiMode::Implementation` if:
    /// - `implementation_state` is present, AND
    /// - The implementation phase is NOT `Complete`
    ///
    /// Otherwise returns `UiMode::Planning`.
    pub fn workflow_stage(&self) -> UiMode {
        match &self.implementation_state {
            Some(impl_state) if impl_state.phase != ImplementationPhase::Complete => {
                UiMode::Implementation
            }
            _ => UiMode::Planning,
        }
    }

    /// Returns true if implementation workflow is currently active.
    /// This is a convenience wrapper around `workflow_stage()`.
    #[allow(dead_code)]
    pub fn implementation_active(&self) -> bool {
        self.workflow_stage() == UiMode::Implementation
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
