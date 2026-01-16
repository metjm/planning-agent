pub mod fixing;
pub mod planning;
mod review_parser;
mod review_prompts;
pub mod reviewing;
pub mod revising;
pub mod summary;
pub mod verification;

pub use fixing::run_fixing_phase;
pub use planning::run_planning_phase_with_context;
pub use reviewing::{
    aggregate_reviews, merge_feedback, run_multi_agent_review_with_context, write_feedback_files,
    ReviewFailure, ReviewResult,
};
pub use revising::run_revision_phase_with_context;
pub use summary::spawn_summary_generation;
pub use verification::{
    parse_verification_verdict, run_verification_phase, VerificationVerdictResult,
};

/// Constructs the conversation key for planning and revision phases.
/// Both phases MUST use this function to ensure conversation continuity.
pub fn planning_conversation_key(agent_name: &str) -> String {
    format!("planning/{}", agent_name)
}

/// Constructs the conversation key for reviewing phases.
/// Uses a separate namespace to prevent collision with planning conversations.
pub fn reviewing_conversation_key(display_id: &str) -> String {
    format!("reviewing/{}", display_id)
}
