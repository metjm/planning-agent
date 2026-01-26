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
            let inputs_content: Vec<String> = self
                .inputs
                .iter()
                .map(|(label, value)| xml_tag(label, value))
                .collect();
            sections.push(xml_tag_raw("inputs", &inputs_content.join("\n")));
        }

        // Constraints
        if !self.constraints.is_empty() {
            let constraints_content = self
                .constraints
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
#[path = "tests/prompt_format_tests.rs"]
mod tests;
