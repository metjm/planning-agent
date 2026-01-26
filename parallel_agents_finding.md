# Architectural Finding: Parallel Agent Implementations

## 1. Summary

A significant architectural flaw exists within the `src/agents/` directory. The implementations for the `claude`, `codex`, and `gemini` agents follow a parallel, nearly identical structure. This duplication of code violates the "Don't Repeat Yourself" (DRY) principle, leading to increased maintenance costs, a higher risk of bugs, and poor code scalability.

The core logic for agent execution—including process setup, timeout handling, logging, and result parsing—is copied across each agent's module. The primary variation between them is the construction of the command-line arguments, which is specific to each agent's underlying CLI tool.

## 2. Analysis of Duplicated Components

The investigation confirmed that the following files are near-copies of each other, with only minor name changes and agent-specific logic in one or two methods.

**Primary Files:**
- `src/agents/claude/agent.rs`
- `src/agents/codex/agent.rs`
- `src/agents/gemini/agent.rs`

**Key Duplicated Elements:**

- **Struct Definition**: The main struct in each file (`ClaudeAgent`, `CodexAgent`, `GeminiAgent`) has the exact same set of fields: `name`, `config`, `working_dir`, `activity_timeout`, `overall_timeout`.
- **`new()` Method**: The constructor is identical in all three modules.
- **`execute_streaming_internal()` Method**: This core method contains the bulk of the duplicated logic. It prepares the runner configuration, attaches loggers, manages cancellation, and invokes the `run_agent_process` function. The only substantive difference is the hard-coded instantiation of the agent-specific parser (`ClaudeParser`, `CodexParser`, `GeminiParser`).
- **Logging Helpers**: Functions like `log_start` are replicated in each module with very minor variations.
- **Test Harness**: The test code within each module is also highly duplicative, using the same helpers to construct agent and context objects.

The only function with significant, necessary variation is **`build_command()`**, which is responsible for assembling the `tokio::process::Command` specific to each agent.

## 3. Impact

This parallel implementation pattern has several negative consequences:

- **High Maintenance Overhead**: Any bug fix, feature enhancement, or refactoring in the core agent execution logic must be manually replicated across all three modules. Forgetting to update one can lead to inconsistent behavior.
- **Increased Risk of Bugs**: The copy-paste nature of this design makes it easy to introduce subtle errors. A developer might fix a bug in one agent's implementation but miss the others, leading to regressions or inconsistent behavior between agents.
- **Poor Scalability**: Adding a new agent (e.g., `mistral`) would require copying and pasting one of the existing modules again, further compounding the technical debt.
- **Reduced Readability**: The duplication obscures the true intent of the module. Developers must read through a large amount of boilerplate to find the small part that is actually unique to the agent.

## 4. Recommendation

I recommend refactoring the agent implementations to abstract the common logic into a single, reusable structure. This will centralize the core execution logic and leave only the agent-specific parts in their respective modules.

My proposed plan is as follows:

1.  **Create a Generic `Agent` Struct**: Introduce a new, generic `Agent` struct in a shared module like `src/agents/base.rs`. This struct would contain the common data (config, timeouts, etc.) and the shared `execute_streaming_internal` logic. I had already created an empty file for this at this path before being asked to pivot to documentation.

2.  **Define a `CommandBuilder` Trait**: Create a trait that defines the contract for agent-specific logic.

    ```rust
    // In src/agents/base.rs
    pub trait CommandBuilder {
        fn build_command(&self, config: &AgentConfig, prepared: &PreparedPrompt, context: Option<&AgentContext>) -> Command;
    }
    ```

3.  **Refactor the Generic Agent**: The generic `Agent`'s execution logic would be parameterized over a parser that implements the `StreamingParser` trait and a command builder that implements the `CommandBuilder` trait.

4.  **Simplify Existing Agent Modules**: Each agent module (`claude`, `codex`, `gemini`) would be reduced to:
    - A simple struct (e.g., `ClaudeCommandBuilder`).
    - An implementation of the `CommandBuilder` trait for that struct, containing the unique `build_command` logic.
    - A top-level "factory" function or struct that constructs the generic `Agent` with the correct `CommandBuilder` and `StreamingParser`.

This approach will eliminate over 90% of the duplicated code, making the agent subsystem more robust, easier to maintain, and simpler to extend in the future.
