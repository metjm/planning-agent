//! XML-style prompt formatting helpers.
//!
//! Provides consistent XML-tag-based prompt structure for all LLM interactions.
//! This module ensures prompts have clearly separated sections with instructions at the top.

/// Wraps content in an XML tag with the given name.
///
/// # Example
/// ```
/// use planning_agent::prompt_format::xml_tag;
/// assert_eq!(xml_tag("phase", "planning"), "<phase>planning</phase>");
/// ```
pub fn xml_tag(name: &str, content: &str) -> String {
    format!("<{}>{}</{}>", name, content, name)
}

/// Wraps content in an XML tag, treating the content as raw (no escaping).
/// Use this for sections containing literal XML tags that must be preserved (e.g., output format examples).
///
/// # Example
/// ```
/// use planning_agent::prompt_format::xml_tag_raw;
/// let output = xml_tag_raw("output-format", "<plan-feedback>...</plan-feedback>");
/// assert!(output.contains("<plan-feedback>"));
/// ```
pub fn xml_tag_raw(name: &str, content: &str) -> String {
    format!("<{}>\n{}\n</{}>", name, content.trim(), name)
}

/// Escapes XML special characters in user-supplied values.
///
/// # Example
/// ```
/// use planning_agent::prompt_format::xml_escape;
/// assert_eq!(xml_escape("a < b & c"), "a &lt; b &amp; c");
/// ```
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Builder for constructing XML-structured prompts.
///
/// Enforces consistent section ordering: phase, instructions, context/inputs, constraints, tools, output-format.
pub struct PromptBuilder {
    phase: Option<String>,
    instructions: Option<String>,
    context: Option<String>,
    inputs: Vec<(String, String)>,
    constraints: Vec<String>,
    tools: Option<String>,
    output_format: Option<String>,
}

impl PromptBuilder {
    /// Creates a new PromptBuilder.
    pub fn new() -> Self {
        Self {
            phase: None,
            instructions: None,
            context: None,
            inputs: Vec::new(),
            constraints: Vec::new(),
            tools: None,
            output_format: None,
        }
    }

    /// Sets the phase name (e.g., "planning", "reviewing").
    pub fn phase(mut self, phase: &str) -> Self {
        self.phase = Some(phase.to_string());
        self
    }

    /// Sets the instructions section.
    pub fn instructions(mut self, instructions: &str) -> Self {
        self.instructions = Some(instructions.to_string());
        self
    }

    /// Sets the context section.
    pub fn context(mut self, context: &str) -> Self {
        self.context = Some(context.to_string());
        self
    }

    /// Adds an input with a label (e.g., "workspace-root", "objective").
    pub fn input(mut self, label: &str, value: &str) -> Self {
        self.inputs.push((label.to_string(), value.to_string()));
        self
    }

    /// Adds a constraint.
    pub fn constraint(mut self, constraint: &str) -> Self {
        self.constraints.push(constraint.to_string());
        self
    }

    /// Sets the tools section.
    pub fn tools(mut self, tools: &str) -> Self {
        self.tools = Some(tools.to_string());
        self
    }

    /// Sets the output format section (raw, for preserving literal XML tags).
    pub fn output_format(mut self, format: &str) -> Self {
        self.output_format = Some(format.to_string());
        self
    }

    /// Builds the complete XML-structured prompt.
    pub fn build(self) -> String {
        let mut sections = Vec::new();

        // Phase comes first
        if let Some(phase) = &self.phase {
            sections.push(xml_tag("phase", phase));
        }

        // Instructions at the top (best practice)
        if let Some(instructions) = &self.instructions {
            sections.push(xml_tag_raw("instructions", instructions));
        }

        // Context
        if let Some(context) = &self.context {
            sections.push(xml_tag_raw("context", context));
        }

        // Inputs
        if !self.inputs.is_empty() {
            let inputs_content: Vec<String> = self.inputs
                .iter()
                .map(|(label, value)| xml_tag(label, value))
                .collect();
            sections.push(xml_tag_raw("inputs", &inputs_content.join("\n")));
        }

        // Constraints
        if !self.constraints.is_empty() {
            let constraints_content = self.constraints
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(xml_tag_raw("constraints", &constraints_content));
        }

        // Tools
        if let Some(tools) = &self.tools {
            sections.push(xml_tag_raw("tools", tools));
        }

        // Output format (raw to preserve literal tags)
        if let Some(output_format) = &self.output_format {
            sections.push(xml_tag_raw("output-format", output_format));
        }

        // Wrap everything in user-prompt root tag
        format!("<user-prompt>\n{}\n</user-prompt>", sections.join("\n"))
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
            .output_format(r#"<plan-feedback>
## Summary
[Your review summary]

## Overall Assessment: [APPROVED or NEEDS REVISION]
</plan-feedback>"#)
            .build();

        assert!(prompt.starts_with("<user-prompt>"));
        assert!(prompt.contains("<phase>reviewing</phase>"));
        assert!(prompt.contains("<workspace-root>/workspaces/myproject</workspace-root>"));
        assert!(prompt.contains("<plan-feedback>"));
        assert!(prompt.contains("Overall Assessment:"));
    }
}
