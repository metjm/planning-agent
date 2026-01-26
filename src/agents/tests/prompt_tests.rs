use super::*;

#[test]
fn test_prepare_prompt_claude_with_system() {
    let request = PromptRequest::new("user prompt".to_string())
        .with_system_prompt("system prompt".to_string())
        .with_max_turns(10);

    let prepared = prepare_prompt(request, AgentCapabilities::Claude);

    assert_eq!(prepared.prompt, "user prompt");
    assert_eq!(
        prepared.system_prompt_arg,
        Some("system prompt".to_string())
    );
    assert_eq!(prepared.max_turns_arg, Some(10));
}

#[test]
fn test_prepare_prompt_codex_merges_system() {
    let request = PromptRequest::new("user prompt".to_string())
        .with_system_prompt("system prompt".to_string())
        .with_max_turns(10);

    let prepared = prepare_prompt(request, AgentCapabilities::Codex);

    assert!(prepared.prompt.contains("<system-context>"));
    assert!(prepared.prompt.contains("system prompt"));
    assert!(prepared.prompt.contains("user prompt"));
    assert_eq!(prepared.system_prompt_arg, None);
    assert_eq!(prepared.max_turns_arg, None);
}

#[test]
fn test_prepare_prompt_no_system() {
    let request = PromptRequest::new("user prompt".to_string());

    let prepared = prepare_prompt(request, AgentCapabilities::Codex);

    assert_eq!(prepared.prompt, "user prompt");
    assert_eq!(prepared.system_prompt_arg, None);
}
