use super::*;
use crate::config::SessionPersistenceConfig;

#[test]
fn test_agent_type_from_config_claude() {
    let config = AgentConfig {
        command: "claude".to_string(),
        args: vec!["-p".to_string()],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let agent = AgentType::from_config("claude", &config, PathBuf::from(".")).unwrap();
    assert_eq!(agent.name(), "claude");
}

#[test]
fn test_agent_type_from_config_codex() {
    let config = AgentConfig {
        command: "codex".to_string(),
        args: vec!["exec".to_string()],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let agent = AgentType::from_config("codex", &config, PathBuf::from(".")).unwrap();
    assert_eq!(agent.name(), "codex");
}

#[test]
fn test_agent_type_from_config_gemini() {
    let config = AgentConfig {
        command: "gemini".to_string(),
        args: vec!["-p".to_string()],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let agent = AgentType::from_config("gemini", &config, PathBuf::from(".")).unwrap();
    assert_eq!(agent.name(), "gemini");
}

#[test]
fn test_agent_type_from_config_unknown() {
    let config = AgentConfig {
        command: "unknown".to_string(),
        args: vec![],
        allowed_tools: vec![],
        session_persistence: SessionPersistenceConfig::default(),
    };
    let result = AgentType::from_config("unknown", &config, PathBuf::from("."));
    assert!(result.is_err());
}
