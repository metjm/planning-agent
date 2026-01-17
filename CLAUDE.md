# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Build
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Run a specific test
cargo test test_name

# Run with clippy (all warnings are denied)
cargo clippy

# Install locally
cargo install --path .
```

## Architecture Overview

Planning-agent is a TUI/headless tool for iterative AI-powered implementation planning. It orchestrates multiple AI agents (Claude, Codex, Gemini) through a structured workflow.

### Workflow State Machine

The core workflow follows this cycle:
- **Planning** → **Reviewing** → (approved) → **Complete** → User Approval
- **Planning** → **Reviewing** → (rejected) → **Revising** → **Reviewing** (loop)

Max iterations (default 3) prevents infinite revision loops.

### Key Module Structure

**Entry Points** (`src/main.rs`):
- TUI mode: `run_tui()` - interactive terminal UI
- Headless mode: `run_headless()` - non-interactive execution
- MCP server mode: internal mode for review feedback collection

**Workflow Engine** (`src/app/workflow/`):
- `mod.rs` - Main workflow loop with `run_workflow_with_config()`
- `planning.rs`, `reviewing.rs`, `revising.rs`, `completion.rs` - Phase handlers
- Uses `tokio::select!` pattern for concurrent channel handling (see module doc comment for critical pattern)

**Agent Abstraction** (`src/agents/`):
- `AgentType` enum wraps Claude, Codex, and Gemini agents
- Each agent (in `claude/`, `codex/`, `gemini/`) handles CLI invocation and output parsing
- `runner.rs` - Common streaming execution logic
- `protocol.rs` - Agent output protocol handling

**Phase Logic** (`src/phases/`):
- `planning.rs`, `reviewing.rs`, `revising.rs` - Phase-specific agent invocation
- `review_parser.rs` - Parses `<plan-feedback>` tags from reviewer output
- `verification.rs` - Post-implementation verification workflow

**TUI Layer** (`src/tui/`):
- `session/` - Session state management with snapshot/restore support
- `ui/` - Ratatui-based UI components (panels, overlays, stats)
- `event.rs` - Event handling with `SessionEventSender` for cross-task communication
- `embedded_terminal.rs` - PTY-based terminal for Claude Code handoff

**Configuration** (`src/config.rs`):
- `WorkflowConfig` - Loaded from `workflow.yaml` or `--config`
- Defines agents, phase assignments, aggregation modes, failure policies

**State Management** (`src/state.rs`):
- `State` - Workflow state with phase, iteration, plan paths
- `Phase` enum: Planning, Reviewing, Revising, Complete
- Persisted to `~/.planning-agent/state/<wd-hash>/<feature>.json`

**Prompt Handling** (`src/agents/prompt.rs`):
- `PreparedPrompt` - Centralized prompt preparation for all agent types
- `AgentCapabilities` - Defines what each agent CLI supports (system prompts, max turns)
- For Claude: system prompts passed via `--append-system-prompt`
- For Codex/Gemini: system prompts merged into user prompt within `<system-context>` tags

**Session Logging** (`src/session_logger.rs`):
- `SessionLogger` - Unified logging for session-scoped events
- `LogCategory` enum: Workflow, Agent, State, Ui, System
- All timestamps in UTC ISO 8601 format for consistency

### Data Storage

All data stored under `~/.planning-agent/`:

**Session-Centric Structure** (new sessions):
```
~/.planning-agent/sessions/<session-id>/
├── plan.md              # Implementation plan
├── feedback_1.md        # Review feedback (round 1)
├── feedback_2.md        # Review feedback (round 2)
├── state.json           # Workflow state
├── session.json         # Session snapshot (for resume)
├── session_info.json    # Lightweight metadata for listing
├── logs/
│   ├── session.log      # Main session log
│   └── agent-stream.log # Raw agent output
└── diagnostics/         # Failure bundles
```

**Legacy Structure** (backward compatibility):
- `plans/` - Legacy plan folders with timestamp-uuid prefix
- `sessions/*.json` - Legacy snapshot files
- `state/<wd-hash>/` - Per-project workflow state
- `logs/<wd-hash>/` - Workflow and agent stream logs

The system reads from both structures for backward compatibility with existing sessions.

### Agent Protocol

Agents output streaming JSON. Review feedback must use `<plan-feedback verdict="approve|reject">` tags for structured parsing. MCP (Model Context Protocol) servers are injected for review feedback collection.

### Naming Conventions: Sessions vs Conversations

**IMPORTANT**: There are two distinct concepts that must not be confused:

| Term | Meaning | Storage |
|------|---------|---------|
| **Workflow Session** | planning-agent's orchestration unit | `~/.planning-agent/sessions/<uuid>/` |
| **Agent Conversation** | Claude/Codex/Gemini's persistent chat context | Managed by each agent's CLI |

**Variable naming conventions:**
- `workflow_session_id` - planning-agent's session identifier
- `conversation_id` - AI agent's conversation/thread ID for resume
- `agent_conversations` - Map of agent name → conversation state
- `AgentConversationState` - State for an agent's conversation
- `ResumeStrategy::ConversationResume` - Resume using captured conversation ID

**Why this matters:**
- Workflow sessions are what users see in the session browser
- Agent conversations enable context continuity between planning→revising phases
- Confusing these leads to bugs like passing workflow IDs to agent resume flags

### Channel Pattern (Critical)

When awaiting channel receives in workflow code, always use `tokio::select!` to check both `approval_rx` AND `control_rx`. Single-channel awaits cause freeze bugs on quit. See `src/app/workflow/mod.rs` header comment for the pattern.

### File Line Limits

The build enforces a 750-line limit per file (see `build.rs`). When a file exceeds this limit:

- **DO**: Extract related functions into a new module (e.g., `workflow_lifecycle.rs` from `events.rs`)
- **DON'T**: Compress code with hacky tricks like removing comments, shortening names, or condensing logic

Proper extraction maintains readability and creates logical module boundaries.

### Code Quality Standards

**No shortcuts. No laziness. No excuses.**

This is non-negotiable. Every problem must be properly investigated and fixed. When something breaks, you fix it correctly - you do not:

- Dismiss failures as "flaky" or "pre-existing" without investigation
- Claim something is an "environment issue" to avoid doing the work
- Create wrapper functions or shims to avoid proper refactoring
- Mark tests as ignored instead of fixing them
- Add `#[allow(...)]` attributes to silence legitimate warnings
- Hand-wave problems away with vague explanations

When tests fail, you investigate why they fail and fix the root cause. When refactoring is needed, you do the refactoring properly. When something is broken, you take ownership and fix it.

**Specific requirements:**

- **DO**: Properly extract code into new modules with correct visibility (`pub(crate)`)
- **DO**: Update all call sites when refactoring function signatures
- **DO**: Write proper tests that use the actual API, not test-only wrappers
- **DO**: Investigate every test failure and fix the underlying issue
- **DO**: Take the time to understand problems before proposing solutions

**No Timelines in Plans:**

Plans must not include timelines, schedules, dates, durations, or time estimates. Focus on technical scope, sequencing, and verification only. Examples to reject: "in two weeks", "Phase 1: Week 1-2", "Q1 delivery", "Sprint 1", "by end of day".

The lazy path is never acceptable. If a task requires significant effort, that effort must be made. Quality and correctness are not optional.
