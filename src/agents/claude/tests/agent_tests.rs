use super::*;
use crate::agents::AgentContext;
use crate::config::SessionPersistenceConfig;
use crate::session_daemon::SessionLogger;
use crate::tui::SessionEventSender;
use std::sync::Arc;
use tokio::sync::mpsc;

fn make_agent(session_persistence_enabled: bool) -> ClaudeAgent {
    let config = AgentConfig {
        command: "claude".to_string(),
        args: vec!["-p".to_string()],
        allowed_tools: vec!["Read".to_string()],
        session_persistence: SessionPersistenceConfig {
            enabled: session_persistence_enabled,
            strategy: ResumeStrategy::ConversationResume,
        },
    };
    ClaudeAgent::new("claude".to_string(), config, PathBuf::from("."))
}

fn make_context(conversation_id: Option<String>, resume_strategy: ResumeStrategy) -> AgentContext {
    // Create a test session_id in the proper format
    let session_id = format!("test-{}", uuid::Uuid::new_v4());
    let session_logger = Arc::new(SessionLogger::new(&session_id).expect("test logger"));

    // Create a channel for the sender (we won't use it, just need to satisfy types)
    let (tx, _rx) = mpsc::unbounded_channel();
    let session_sender = SessionEventSender::new(0, 0, tx);

    AgentContext {
        session_sender,
        phase: "Testing".to_string(),
        conversation_id,
        resume_strategy,
        cancel_rx: None,
        session_logger,
    }
}

fn make_prepared_prompt() -> PreparedPrompt {
    PreparedPrompt {
        prompt: "test prompt".to_string(),
        system_prompt_arg: None,
        max_turns_arg: None,
    }
}

fn get_args(cmd: &Command) -> Vec<String> {
    let cmd_debug = format!("{:?}", cmd);
    // Parse args from debug output - crude but works for testing
    cmd_debug
        .split('"')
        .filter(|s| !s.is_empty() && !s.contains('=') && !s.contains('{') && !s.contains('}'))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "," && s != " ")
        .collect()
}

#[test]
fn test_claude_agent_new() {
    let config = AgentConfig {
        command: "claude".to_string(),
        args: vec!["-p".to_string()],
        allowed_tools: vec!["Read".to_string()],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let agent = ClaudeAgent::new("claude".to_string(), config, PathBuf::from("."));
    assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
    assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
}

#[test]
fn test_build_command_with_resume_when_conversation_id_present() {
    let agent = make_agent(true);
    let prepared = make_prepared_prompt();
    let ctx = make_context(
        Some("abc-123-def".to_string()),
        ResumeStrategy::ConversationResume,
    );
    let cmd = agent.build_command(&prepared, Some(&ctx));
    let args = get_args(&cmd);

    // Should contain --resume followed by the conversation ID
    assert!(
        args.contains(&"--resume".to_string()),
        "Command should include --resume flag. Args: {:?}",
        args
    );
    assert!(
        args.contains(&"abc-123-def".to_string()),
        "Command should include conversation ID. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_stateless() {
    let agent = make_agent(true);
    let prepared = make_prepared_prompt();
    let ctx = make_context(
        Some("abc-123-def".to_string()),
        ResumeStrategy::Stateless, // Stateless strategy
    );
    let cmd = agent.build_command(&prepared, Some(&ctx));
    let args = get_args(&cmd);

    // Should NOT contain --resume
    assert!(
        !args.contains(&"--resume".to_string()),
        "Command should NOT include --resume with Stateless strategy. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_no_conversation_id() {
    let agent = make_agent(true);
    let prepared = make_prepared_prompt();
    let ctx = make_context(
        None, // No conversation ID yet
        ResumeStrategy::ConversationResume,
    );
    let cmd = agent.build_command(&prepared, Some(&ctx));
    let args = get_args(&cmd);

    // Should NOT contain --resume (no ID to resume)
    assert!(
        !args.contains(&"--resume".to_string()),
        "Command should NOT include --resume without conversation ID. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_persistence_disabled() {
    let agent = make_agent(false); // Persistence disabled
    let prepared = make_prepared_prompt();
    let ctx = make_context(
        Some("abc-123-def".to_string()),
        ResumeStrategy::ConversationResume,
    );
    let cmd = agent.build_command(&prepared, Some(&ctx));
    let args = get_args(&cmd);

    // Should NOT contain --resume (persistence disabled)
    assert!(
        !args.contains(&"--resume".to_string()),
        "Command should NOT include --resume when persistence disabled. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_no_context() {
    let agent = make_agent(true);
    let prepared = make_prepared_prompt();
    let cmd = agent.build_command(&prepared, None); // No context
    let args = get_args(&cmd);

    // Should NOT contain --resume (no context)
    assert!(
        !args.contains(&"--resume".to_string()),
        "Command should NOT include --resume without context. Args: {:?}",
        args
    );
}
