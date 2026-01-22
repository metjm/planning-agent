pub mod fixing;
pub mod implementation;
pub mod implementation_review;
pub mod planning;
mod review_parser;
mod review_prompts;
pub mod review_schema;
pub mod reviewing;
pub mod revising;
pub mod summary;
pub mod verdict;
pub mod verification;

pub use fixing::run_fixing_phase;
pub use planning::run_planning_phase_with_context;
pub use reviewing::{
    aggregate_reviews, merge_feedback, run_multi_agent_review_with_context, write_feedback_files,
    ReviewFailure, ReviewResult,
};
pub use revising::run_revision_phase_with_context;
pub use summary::spawn_summary_generation;
// Re-export review schema types for potential external use
#[allow(unused_imports)]
pub use review_schema::{ReviewVerdict, SubmittedReview};
// Re-export verdict types for use by verification and implementation phases
#[allow(unused_imports)]
pub use verdict::{
    extract_implementation_feedback, extract_verification_feedback, parse_verification_verdict,
    VerificationVerdictResult,
};
pub use verification::run_verification_phase;
// Re-export implementation phase types
#[allow(unused_imports)]
pub use implementation::{run_implementation_phase, ImplementationResult};
// Re-export implementation-review phase types
#[allow(unused_imports)]
pub use implementation_review::{run_implementation_review_phase, ImplementationReviewResult};

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

/// Constructs the conversation key for implementation phases.
/// Uses a separate namespace to prevent collision with planning/reviewing conversations.
pub fn implementing_conversation_key(agent_name: &str) -> String {
    format!("implementing/{}", agent_name)
}

/// Constructs the conversation key for implementation-review phases.
/// Uses a separate namespace to prevent collision with other conversations.
pub fn implementation_reviewing_conversation_key(agent_name: &str) -> String {
    format!("implementation-reviewing/{}", agent_name)
}
