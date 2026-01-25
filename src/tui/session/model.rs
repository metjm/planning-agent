use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub status: TodoStatus,
    pub active_form: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub agent_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultSummary {
    pub first_line: String,
    pub line_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    Write,
    Bash,
    Search,
    Other(String),
}

impl ToolKind {
    pub fn from_display_name(display_name: &str) -> Self {
        let normalized = display_name.trim().to_ascii_lowercase();
        if normalized == "read" || normalized.starts_with("read_") {
            return ToolKind::Read;
        }
        if normalized == "write" || normalized.starts_with("write_") {
            return ToolKind::Write;
        }
        if normalized == "bash" || normalized == "shell" || normalized == "run_shell_command" {
            return ToolKind::Bash;
        }
        if normalized == "search" || normalized.starts_with("search_") {
            return ToolKind::Search;
        }
        ToolKind::Other(display_name.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolTimelineEntry {
    Started {
        agent_name: String,
        kind: ToolKind,
        display_name: String,
        input_preview: String,
    },
    Finished {
        agent_name: String,
        kind: ToolKind,
        display_name: String,
        input_preview: String,
        duration_ms: u64,
        is_error: bool,
        result_summary: ToolResultSummary,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunTabEntry {
    Text(ChatMessage),
    Tool(ToolTimelineEntry),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SummaryState {
    #[default]
    None,
    Generating,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTab {
    pub phase: String,
    pub entries: Vec<RunTabEntry>,
    pub scroll_position: usize,

    pub summary_text: String,
    pub summary_scroll: usize,
    pub summary_state: SummaryState,
    #[serde(default)]
    pub summary_spinner_frame: u8,
}

impl RunTab {
    pub fn new(phase: String) -> Self {
        Self {
            phase,
            entries: Vec::new(),
            scroll_position: 0,
            summary_text: String::new(),
            summary_scroll: 0,
            summary_state: SummaryState::None,
            summary_spinner_frame: 0,
        }
    }
}

/// Status of a single reviewer within a round
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReviewerStatus {
    /// Reviewer is currently running
    Running,
    /// Reviewer completed successfully
    Completed {
        approved: bool,
        summary: String,
        duration_ms: u64,
    },
    /// Reviewer failed (execution error, not a rejection)
    Failed { error: String },
}

/// A single reviewer's state within a round
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewerEntry {
    /// Display ID of the reviewer (e.g., "claude", "claude-practices")
    pub display_id: String,
    /// Current status
    pub status: ReviewerStatus,
}

/// A single review round (iteration)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewKind {
    Plan,
    Implementation,
}

impl ReviewKind {
    pub fn label(&self) -> &'static str {
        match self {
            ReviewKind::Plan => "Plan",
            ReviewKind::Implementation => "Implementation",
        }
    }

    pub fn sort_rank(&self) -> u64 {
        match self {
            ReviewKind::Plan => 0,
            ReviewKind::Implementation => 1,
        }
    }
}

/// A single review round (iteration)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewRound {
    /// Review kind for this round (plan or implementation)
    pub kind: ReviewKind,
    /// Round number (1-indexed, matches state.iteration)
    pub round: u32,
    /// Reviewers in this round
    pub reviewers: Vec<ReviewerEntry>,
    /// Aggregate verdict for this round (set when all reviewers complete)
    pub aggregate_verdict: Option<bool>,
}

impl ReviewRound {
    pub fn new(kind: ReviewKind, round: u32) -> Self {
        Self {
            kind,
            round,
            reviewers: Vec::new(),
            aggregate_verdict: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ApprovalMode {
    None,
    AwaitingChoice,
    EnteringFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ApprovalContext {
    #[default]
    PlanApproval,
    ReviewDecision,
    PlanGenerationFailed,
    MaxIterationsReached,
    UserOverrideApproval,
    /// All reviewers failed after retries - prompts for retry, stop, or abort.
    AllReviewersFailed,
    /// Generic workflow failure (agent errors in revising, etc.) - prompts for retry, stop, or abort.
    WorkflowFailure,
}

/// Indicates the target of feedback entry mode.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum FeedbackTarget {
    #[default]
    ApprovalDecline, // Existing: decline with feedback in approval flow
    WorkflowInterrupt, // New: interrupt active workflow with feedback
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum FocusedPanel {
    #[default]
    Output,
    Todos,
    Chat,
    ChatInput,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum InputMode {
    #[default]
    Normal,
    NamingTab,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum SessionStatus {
    #[default]
    InputPending,
    Planning,
    GeneratingSummary,
    AwaitingApproval,
    Stopped, // Cleanly stopped session, can be resumed
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteBlock {
    pub content: String,
    pub start_pos: usize,
    pub line_count: usize,
}

/// Runtime-only modal state for implementation success display.
/// Not serialized - always reset to None on snapshot restore.
#[derive(Debug, Clone)]
pub struct ImplementationSuccessModal {
    pub iterations_used: u32,
}

/// Entry representing a single review file for modal display.
///
/// Entries are sorted by (iteration DESC, agent_name ASC) to show most recent
/// reviews first, with deterministic ordering when multiple agents review
/// the same iteration.
#[derive(Debug, Clone)]
pub struct ReviewModalEntry {
    /// Review kind for this entry
    pub kind: ReviewKind,
    /// Display name shown in tab (e.g., "Round 1 - claude", "Round 2")
    pub display_name: String,
    /// Cached content of the feedback file
    pub content: String,
    /// Sort key for ordering: iteration * 1_000_000_000 + (kind_rank * 1_000_000) + (1_000_000 - agent_ordinal)
    /// Higher values = more recent iteration, then by kind, then by agent
    pub sort_key: u64,
}
