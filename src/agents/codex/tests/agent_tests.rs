use super::*;
use crate::agents::AgentContext;
use crate::config::SessionPersistenceConfig;
use crate::session_daemon::SessionLogger;
use crate::tui::SessionEventSender;
use std::sync::Arc;
use tokio::sync::mpsc;

fn make_agent(session_persistence_enabled: bool) -> CodexAgent {
    let config = AgentConfig {
        command: "codex".to_string(),
        args: vec!["exec".to_string(), "--json".to_string()],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig {
            enabled: session_persistence_enabled,
            strategy: ResumeStrategy::ConversationResume,
        },
    };
    CodexAgent::new("codex".to_string(), config, PathBuf::from("."))
}

fn make_context(conversation_id: Option<String>, resume_strategy: ResumeStrategy) -> AgentContext {
    let session_id = format!("test-{}", uuid::Uuid::new_v4());
    let session_logger = Arc::new(SessionLogger::new(&session_id).expect("test logger"));
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

fn get_args(cmd: &Command) -> Vec<String> {
    let cmd_debug = format!("{:?}", cmd);
    cmd_debug
        .split('"')
        .filter(|s| !s.is_empty() && !s.contains('=') && !s.contains('{') && !s.contains('}'))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "," && s != " ")
        .collect()
}

#[test]
fn test_codex_agent_new() {
    let config = AgentConfig {
        command: "codex".to_string(),
        args: vec!["exec".to_string(), "--json".to_string()],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let agent = CodexAgent::new("codex".to_string(), config, PathBuf::from("."));
    assert_eq!(agent.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
    assert_eq!(agent.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
}

#[test]
fn test_build_command_with_resume_when_conversation_id_present() {
    let agent = make_agent(true);
    let ctx = make_context(
        Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
        ResumeStrategy::ConversationResume,
    );
    let cmd = agent.build_command("test prompt", Some(&ctx));
    let args = get_args(&cmd);

    // Should contain "exec resume [session_id]" sequence
    assert!(
        args.contains(&"resume".to_string()),
        "Command should include resume subcommand. Args: {:?}",
        args
    );
    assert!(
        args.contains(&"019bc838-8e90-7052-b458-3615bee3647a".to_string()),
        "Command should include conversation ID. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_stateless() {
    let agent = make_agent(true);
    let ctx = make_context(
        Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
        ResumeStrategy::Stateless,
    );
    let cmd = agent.build_command("test prompt", Some(&ctx));
    let args = get_args(&cmd);

    assert!(
        !args.contains(&"resume".to_string()),
        "Command should NOT include resume with Stateless strategy. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_no_conversation_id() {
    let agent = make_agent(true);
    let ctx = make_context(None, ResumeStrategy::ConversationResume);
    let cmd = agent.build_command("test prompt", Some(&ctx));
    let args = get_args(&cmd);

    assert!(
        !args.contains(&"resume".to_string()),
        "Command should NOT include resume without conversation ID. Args: {:?}",
        args
    );
}

#[test]
fn test_build_command_no_resume_when_persistence_disabled() {
    let agent = make_agent(false);
    let ctx = make_context(
        Some("019bc838-8e90-7052-b458-3615bee3647a".to_string()),
        ResumeStrategy::ConversationResume,
    );
    let cmd = agent.build_command("test prompt", Some(&ctx));
    let args = get_args(&cmd);

    assert!(
        !args.contains(&"resume".to_string()),
        "Command should NOT include resume when persistence disabled. Args: {:?}",
        args
    );
}
