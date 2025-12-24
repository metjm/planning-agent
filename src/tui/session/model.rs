
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone)]
pub struct TodoItem {
    pub status: TodoStatus,
    pub active_form: String,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub agent_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SummaryState {
    #[default]
    None,
    Generating,
    Ready,
    Error,
}

#[derive(Debug, Clone)]
pub struct RunTab {
    pub phase: String,           
    pub messages: Vec<ChatMessage>,
    pub scroll_position: usize,  

    pub summary_text: String,
    pub summary_scroll: usize,
    pub summary_state: SummaryState,
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

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalMode {

    None,

    AwaitingChoice,

    EnteringFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ApprovalContext {
    #[default]
    PlanApproval,
    ReviewDecision,
    PlanGenerationFailed,   
    MaxIterationsReached,   
    UserOverrideApproval,   
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FocusedPanel {
    #[default]
    Output,
    Chat,  
    Summary,  
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputMode {
    #[default]
    Normal,

    NamingTab,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SessionStatus {
    #[default]
    InputPending,
    Planning,
    GeneratingSummary,
    AwaitingApproval,
    Complete,
    Error,
}

#[derive(Debug, Clone)]
pub struct PasteBlock {

    pub content: String,

    pub start_pos: usize,

    pub line_count: usize,
}
