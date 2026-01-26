use super::*;

#[test]
fn test_parse_error_display() {
    let err = ParseError;
    assert_eq!(format!("{}", err), "Parse error");
}

#[test]
fn test_token_usage_conversion() {
    let agent_usage = AgentTokenUsage {
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 10,
        cache_read_tokens: 5,
    };

    let tui_usage: TokenUsage = agent_usage.clone().into();
    assert_eq!(tui_usage.input_tokens, 100);
    assert_eq!(tui_usage.output_tokens, 50);
    assert_eq!(tui_usage.cache_creation_tokens, 10);
    assert_eq!(tui_usage.cache_read_tokens, 5);

    let back: AgentTokenUsage = tui_usage.into();
    assert_eq!(back.input_tokens, 100);
    assert_eq!(back.output_tokens, 50);
}

#[test]
fn test_agent_output_default() {
    let output = AgentOutput {
        output: "test".to_string(),
        is_error: false,
        conversation_id: None,
        stop_reason: None,
    };
    assert_eq!(output.output, "test");
    assert!(!output.is_error);
}

#[test]
fn test_agent_output_with_conversation_id() {
    let output = AgentOutput {
        output: "test".to_string(),
        is_error: false,
        conversation_id: Some("abc-123".to_string()),
        stop_reason: None,
    };
    assert_eq!(output.conversation_id, Some("abc-123".to_string()));
}
