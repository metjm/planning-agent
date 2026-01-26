use super::*;

#[test]
fn test_xml_tag() {
    assert_eq!(xml_tag("phase", "planning"), "<phase>planning</phase>");
    assert_eq!(xml_tag("name", "test"), "<name>test</name>");
}

#[test]
fn test_xml_tag_raw() {
    let result = xml_tag_raw("output-format", "<plan-feedback>content</plan-feedback>");
    assert!(result.starts_with("<output-format>"));
    assert!(result.ends_with("</output-format>"));
    assert!(result.contains("<plan-feedback>content</plan-feedback>"));
}

#[test]
fn test_xml_escape() {
    assert_eq!(xml_escape("a < b"), "a &lt; b");
    assert_eq!(xml_escape("a > b"), "a &gt; b");
    assert_eq!(xml_escape("a & b"), "a &amp; b");
    assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
    assert_eq!(xml_escape("it's"), "it&apos;s");
    assert_eq!(xml_escape("normal text"), "normal text");
}

#[test]
fn test_prompt_builder_basic() {
    let prompt = PromptBuilder::new()
        .phase("planning")
        .instructions("Create a plan")
        .build();

    assert!(prompt.starts_with("<user-prompt>"));
    assert!(prompt.ends_with("</user-prompt>"));
    assert!(prompt.contains("<phase>planning</phase>"));
    assert!(prompt.contains("<instructions>"));
    assert!(prompt.contains("Create a plan"));
}

#[test]
fn test_prompt_builder_with_inputs() {
    let prompt = PromptBuilder::new()
        .phase("reviewing")
        .input("workspace-root", "/workspace")
        .input("objective", "Ship feature")
        .build();

    assert!(prompt.contains("<inputs>"));
    assert!(prompt.contains("<workspace-root>/workspace</workspace-root>"));
    assert!(prompt.contains("<objective>Ship feature</objective>"));
}

#[test]
fn test_prompt_builder_with_constraints() {
    let prompt = PromptBuilder::new()
        .constraint("Use absolute paths")
        .constraint("Do not modify unrelated files")
        .build();

    assert!(prompt.contains("<constraints>"));
    assert!(prompt.contains("- Use absolute paths"));
    assert!(prompt.contains("- Do not modify unrelated files"));
}

#[test]
fn test_prompt_builder_preserves_output_tags() {
    let prompt = PromptBuilder::new()
        .phase("reviewing")
        .output_format("<plan-feedback>\nYour feedback here\n</plan-feedback>")
        .build();

    assert!(prompt.contains("<output-format>"));
    assert!(prompt.contains("<plan-feedback>"));
    assert!(prompt.contains("</plan-feedback>"));
}

#[test]
fn test_prompt_builder_section_ordering() {
    let prompt = PromptBuilder::new()
        .phase("planning")
        .instructions("Instructions here")
        .context("Context here")
        .input("key", "value")
        .constraint("Constraint here")
        .tools("Tools here")
        .output_format("Format here")
        .build();

    // Check that sections appear in the expected order
    let phase_pos = prompt.find("<phase>").unwrap();
    let instructions_pos = prompt.find("<instructions>").unwrap();
    let context_pos = prompt.find("<context>").unwrap();
    let inputs_pos = prompt.find("<inputs>").unwrap();
    let constraints_pos = prompt.find("<constraints>").unwrap();
    let tools_pos = prompt.find("<tools>").unwrap();
    let output_pos = prompt.find("<output-format>").unwrap();

    assert!(phase_pos < instructions_pos);
    assert!(instructions_pos < context_pos);
    assert!(context_pos < inputs_pos);
    assert!(inputs_pos < constraints_pos);
    assert!(constraints_pos < tools_pos);
    assert!(tools_pos < output_pos);
}

#[test]
fn test_prompt_builder_full_example() {
    let prompt = PromptBuilder::new()
        .phase("reviewing")
        .instructions("Review the implementation plan for correctness and completeness.")
        .input("workspace-root", "/workspaces/myproject")
        .input("objective", "Add user authentication")
        .constraint("Use absolute paths for all file references")
        .output_format(
            r#"<plan-feedback>
## Summary
[Your review summary]

## Overall Assessment: [APPROVED or NEEDS REVISION]
</plan-feedback>"#,
        )
        .build();

    assert!(prompt.starts_with("<user-prompt>"));
    assert!(prompt.contains("<phase>reviewing</phase>"));
    assert!(prompt.contains("<workspace-root>/workspaces/myproject</workspace-root>"));
    assert!(prompt.contains("<plan-feedback>"));
    assert!(prompt.contains("Overall Assessment:"));
}
