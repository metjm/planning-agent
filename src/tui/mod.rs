pub mod embedded_terminal;
mod event;
pub mod file_index;
pub mod mention;
pub mod session;
pub mod session_browser;
pub mod slash;
mod tabs;
mod title;
pub mod ui;

pub use session::{ApprovalContext, ApprovalMode, FeedbackTarget, FocusedPanel, InputMode, RunTab, Session, SessionContext, SessionStatus, SummaryState, TodoItem, TodoStatus};
pub use tabs::TabManager;
pub use event::{CancellationError, Event, EventHandler, SessionEventSender, TokenUsage, UserApprovalResponse, WorkflowCommand};
pub use title::TerminalTitleManager;
