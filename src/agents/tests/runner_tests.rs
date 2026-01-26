use super::*;

#[test]
fn test_runner_config_defaults() {
    let config = RunnerConfig::new("test".to_string(), PathBuf::from("."));
    assert_eq!(config.activity_timeout, DEFAULT_ACTIVITY_TIMEOUT);
    assert_eq!(config.overall_timeout, DEFAULT_OVERALL_TIMEOUT);
}

#[test]
fn test_runner_config_custom_timeouts() {
    let config = RunnerConfig::new("test".to_string(), PathBuf::from("."))
        .with_activity_timeout(Duration::from_secs(60))
        .with_overall_timeout(Duration::from_secs(600));
    assert_eq!(config.activity_timeout, Duration::from_secs(60));
    assert_eq!(config.overall_timeout, Duration::from_secs(600));
}

#[test]
fn test_agent_output_to_result() {
    let output = AgentOutput {
        output: "test output".to_string(),
        is_error: false,
        conversation_id: Some("conv-123".to_string()),
        stop_reason: Some("max_turns".to_string()),
    };
    let result: AgentResult = output.into();
    assert_eq!(result.output, "test output");
    assert!(!result.is_error);
    assert_eq!(result.conversation_id, Some("conv-123".to_string()));
    assert_eq!(result.stop_reason, Some("max_turns".to_string()));
}

#[test]
fn test_agent_output_to_result_without_conversation_id() {
    let output = AgentOutput {
        output: "test output".to_string(),
        is_error: false,
        conversation_id: None,
        stop_reason: None,
    };
    let result: AgentResult = output.into();
    assert!(result.conversation_id.is_none());
    assert!(result.stop_reason.is_none());
}
