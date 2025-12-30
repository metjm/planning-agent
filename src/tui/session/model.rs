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
    pub messages: Vec<ChatMessage>,
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
            messages: Vec::new(),
            scroll_position: 0,
            summary_text: String::new(),
            summary_scroll: 0,
            summary_state: SummaryState::None,
            summary_spinner_frame: 0,
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
}

/// Indicates the target of feedback entry mode.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum FeedbackTarget {
    #[default]
    ApprovalDecline,    // Existing: decline with feedback in approval flow
    WorkflowInterrupt,  // New: interrupt active workflow with feedback
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum FocusedPanel {
    #[default]
    Output,
    Todos,
    Chat,
    Summary,
    Implementation,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum InputMode {
    #[default]
    Normal,
    NamingTab,
    ImplementationTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum SessionStatus {
    #[default]
    InputPending,
    Planning,
    GeneratingSummary,
    AwaitingApproval,
    Stopped,  // Cleanly stopped session, can be resumed
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasteBlock {
    pub content: String,
    pub start_pos: usize,
    pub line_count: usize,
}
