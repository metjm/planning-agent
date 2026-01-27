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

## Preferences

- Prefer event-driven updates over polling; avoid long-polling.
- Prefer push-based mechanisms (webhooks/pub-sub, server push via SSE/WebSocket) and file/DB change notifications.
- Prefer user-initiated refresh with cache validation (ETag/If-Modified-Since) over background polling.

## Architecture Overview

Planning-agent is a TUI/headless tool for iterative AI-powered implementation planning. It orchestrates multiple AI agents (Claude, Codex, Gemini) through a structured workflow.

### Workflow Phases

The core workflow follows this cycle:

- **Planning** → **Reviewing** → (approved) → **Complete** → User Approval
- **Planning** → **Reviewing** → (rejected) → **Revising** → **Reviewing** (loop)

Max iterations (default 3) prevents infinite revision loops.

## CQRS/Event Sourcing Architecture

**This is the most important section. Read it carefully.**

The workflow uses a strict CQRS (Command Query Responsibility Segregation) architecture with event sourcing. All state changes go through the domain actor.

### Core Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `WorkflowCommand` | `src/domain/cqrs/commands.rs` | Intent to change state |
| `WorkflowEvent` | `src/domain/cqrs/events.rs` | Facts that happened (immutable) |
| `WorkflowAggregate` | `src/domain/cqrs/mod.rs` | Validates commands, emits events |
| `WorkflowView` | `src/domain/view.rs` | Read-only projection for queries |
| `WorkflowActor` | `src/domain/actor.rs` | Serializes command execution (ractor) |
| `FileEventStore` | `src/event_store/file_store.rs` | Persists events to JSONL |

### The Golden Rule: No Direct State Mutation

**NEVER mutate workflow state directly. Always dispatch commands.**

```rust
// WRONG - Direct mutation (this pattern was deleted)
state.phase = Phase::Reviewing;
state.iteration += 1;
state.save(&path)?;

// CORRECT - Dispatch command to actor
dispatch_domain_command(
    &actor_ref,
    WorkflowCommand::PlanningCompleted { plan_path: PlanPath(path) },
    &session_logger,
).await;
```

### How State Changes Flow

```
1. Workflow code dispatches WorkflowCommand
       ↓
2. WorkflowActor receives command
       ↓
3. WorkflowAggregate.handle() validates and emits WorkflowEvent(s)
       ↓
4. Events persisted to ~/.planning-agent/sessions/<id>/events.jsonl
       ↓
5. WorkflowQuery.dispatch() updates WorkflowView projection
       ↓
6. View broadcast via watch channel to TUI
```

### Reading State: Use WorkflowView

All workflow phases receive `&WorkflowView` (read-only). Extract what you need:

```rust
// Reading from view (correct)
let phase = view.planning_phase.unwrap_or(Phase::Planning);
let iteration = view.iteration.unwrap_or_default().0;
let plan_path = view.plan_path.as_ref().map(|p| p.0.clone());
let feature_name = view.feature_name.as_ref().map(|f| f.0.as_str());
```

### Available Commands

Planning phase:
- `CreateWorkflow` - Initialize new workflow
- `StartPlanning` - Begin planning
- `PlanningCompleted { plan_path }` - Plan file written

Review phase:
- `ReviewCycleStarted { mode, reviewers }` - Begin review cycle
- `ReviewerApproved { reviewer_id }` - Single reviewer approved
- `ReviewerRejected { reviewer_id, feedback_path }` - Single reviewer rejected
- `ReviewCycleCompleted { approved }` - All reviewers done

Revision phase:
- `RevisingStarted { feedback_summary }` - Begin revision
- `RevisionCompleted { plan_path }` - Revised plan written

User decisions:
- `UserApproved` - User accepts plan
- `UserDeclined` - User rejects plan
- `UserAborted { reason }` - User cancels workflow
- `UserOverrideApproval` - User overrides AI rejection
- `UserRequestedImplementation` - User wants implementation

Implementation phase:
- `ImplementationStarted { max_iterations }`
- `ImplementationRoundStarted { iteration }`
- `ImplementationRoundCompleted`
- `ImplementationReviewCompleted { verdict, feedback }`
- `ImplementationAccepted` / `ImplementationDeclined` / `ImplementationCancelled`

Metadata:
- `RecordAgentConversation { agent_id, conversation_id, ... }`
- `RecordInvocation { agent_id, phase, ... }`
- `RecordFailure { failure }`
- `AttachWorktree { worktree_state }`

### DO NOT

- **DO NOT** add fields to track state outside the aggregate
- **DO NOT** create new state structs that bypass WorkflowView
- **DO NOT** persist state via direct file writes (use commands)
- **DO NOT** read state from files directly (use WorkflowView)
- **DO NOT** add `&mut` parameters to phase functions for state
- **DO NOT** create shims or wrappers around WorkflowView

### Strong Types

Always use domain types, never primitives:

```rust
// WRONG
let session_id: String = uuid.to_string();
let iteration: u32 = 1;
let path: PathBuf = plan_path;

// CORRECT
let workflow_id: WorkflowId = WorkflowId(uuid);
let iteration: Iteration = Iteration::first();
let path: PlanPath = PlanPath(plan_path);
```

Key types in `src/domain/types.rs`:
- `WorkflowId`, `AgentId`, `ConversationId` - Identity types
- `PlanPath`, `FeedbackPath`, `WorkingDir` - Path wrappers
- `Objective`, `FeatureName` - Content wrappers
- `Iteration`, `MaxIterations` - Numeric wrappers
- `Phase`, `ImplementationPhase` - State enums
- `FeedbackStatus`, `ImplementationVerdict` - Result enums

## Key Module Structure

**Entry Points** (`src/main.rs`):
- TUI mode: `run_tui()` - interactive terminal UI
- Headless mode: `run_headless()` - non-interactive execution

**Workflow Engine** (`src/app/workflow/`):
- `mod.rs` - Main workflow loop, actor spawning, command dispatch
- `planning.rs`, `reviewing.rs`, `revising.rs`, `completion.rs` - Phase handlers
- All phases receive `&WorkflowView` and dispatch commands via `WorkflowPhaseContext`

**Domain Layer** (`src/domain/`):
- `cqrs/` - Aggregate, commands, events, query handler
- `actor.rs` - WorkflowActor (ractor-based)
- `view.rs` - WorkflowView projection
- `types.rs` - Strong domain types
- `input.rs` - WorkflowInput (New/Resume) for initialization

**Event Store** (`src/event_store/`):
- `file_store.rs` - JSONL event persistence with snapshots
- Uses `fs2` for file locking, atomic writes

**Phase Logic** (`src/phases/`):
- `planning.rs`, `reviewing.rs`, `revising.rs` - Agent invocation
- All functions take `&WorkflowView`, never mutate state

**TUI Layer** (`src/tui/`):
- `session/mod.rs` - Session with `workflow_view: Option<WorkflowView>`
- `event.rs` - `Event::SessionViewUpdate` carries boxed `WorkflowView`
- `session_event_sender.rs` - `send_view_update()` broadcasts view changes

**Agent Abstraction** (`src/agents/`):
- `AgentType` enum wraps Claude, Codex, and Gemini agents
- `runner.rs` - Common streaming execution logic

## Data Storage

All data stored under `~/.planning-agent/`:

```
~/.planning-agent/sessions/<session-id>/
├── events.jsonl         # Event log (append-only, source of truth)
├── snapshot.json        # Aggregate snapshot (optimization)
├── plan.md              # Implementation plan
├── feedback_1.md        # Review feedback (round 1)
├── session.json         # UI snapshot for resume
├── session_info.json    # Lightweight metadata for listing
└── logs/
    ├── session.log      # Main session log
    └── agent-stream.log # Raw agent output
```

**Event Log Format** (`events.jsonl`):
```json
{"aggregate_id":"uuid","sequence":1,"event_type":"WorkflowCreated","event":{...}}
{"aggregate_id":"uuid","sequence":2,"event_type":"PlanningCompleted","event":{...}}
```

## Channel Pattern (Critical)

When awaiting channel receives in workflow code, always use `tokio::select!` to check both `approval_rx` AND `control_rx`. Single-channel awaits cause freeze bugs on quit.

```rust
// CORRECT - Always select on both channels
tokio::select! {
    Some(cmd) = control_rx.recv() => {
        match cmd {
            WorkflowCommand::Stop => return Ok(WorkflowResult::Stopped),
            WorkflowCommand::Interrupt { feedback } => { ... }
        }
    }
    response = approval_rx.recv() => {
        // Handle approval
    }
}

// WRONG - Will freeze on quit
let response = approval_rx.recv().await;
```

## Workflow Phase Boundaries (Critical)

Phase handlers (`planning.rs`, `reviewing.rs`, `revising.rs`) do their core work and dispatch commands. The main workflow loop (`mod.rs`) handles all user-facing decisions.

### Phase Handlers Dispatch and Return

When a phase handler reaches a decision point requiring user input, it should dispatch a command to transition to a "waiting" phase and **return immediately**:

```rust
// CORRECT - Dispatch phase transition and return, let main loop handle decision
if iteration >= max_iterations {
    context.dispatch_command(DomainCommand::PlanningMaxIterationsReached).await;
    return Ok(None);  // Main loop will handle AwaitingPlanningDecision
}

// WRONG - Handling user decision inline in phase code
if iteration >= max_iterations {
    context.dispatch_command(DomainCommand::PlanningMaxIterationsReached).await;
    let decision = await_max_iterations_decision(...).await?;  // NO!
    match decision { ... }  // This creates duplicate handling
}
```

### Single Handler for User Decisions

Each user-facing decision type has exactly ONE handler in the main workflow loop:

| Decision Type | Phase | Handler Location |
|--------------|-------|------------------|
| Max iterations reached | `AwaitingPlanningDecision` | `mod.rs` only |
| Plan approval | `Complete` | `mod.rs` only |
| Implementation decision | `AwaitingDecision` | `mod.rs` only |

**DO NOT** call phase-transition decision functions from phase handlers. In particular:
- `await_max_iterations_decision` - **only in `mod.rs`** (enforced by build script)

This function handles `AwaitingPlanningDecision` which has a handler in the main loop. Calling it from phase handlers creates duplicate modal displays.

Other decision functions (`wait_for_review_decision`, `wait_for_all_reviewers_failed_decision`, `wait_for_workflow_failure_decision`) may be called from phase handlers as they handle within-phase decisions, not phase transitions.

## Naming Conventions

### Sessions vs Conversations

| Term | Meaning | Storage |
|------|---------|---------|
| **Workflow Session** | planning-agent's orchestration unit | `~/.planning-agent/sessions/<uuid>/` |
| **Agent Conversation** | Claude/Codex/Gemini's chat context | Managed by each agent's CLI |

Variable naming:
- `workflow_session_id` - planning-agent's session ID
- `conversation_id` - AI agent's conversation/thread ID
- `agent_conversations` - Map of agent → conversation state

## Build Enforcement

The build script (`build.rs`) enforces:

1. **No #[ignore] Tests** - Tests run or get deleted
2. **No Silent Test Skips** - No early returns to skip tests
3. **No Nested Tokio Runtimes** - Use async, not thread spawn + Runtime::new()
4. **Serial Tests for Env Mutations** - Use `#[serial]` for env var changes
5. **No #[allow(dead_code)]** - Delete unused code
6. **Code Formatting** - Must pass `cargo fmt --check`
7. **Max 10 Files Per Folder** - Split large modules
8. **Max 750 Lines Per File** - Extract into submodules
9. **Tests in Test Folders** - Use `#[path = "tests/foo.rs"]` pattern
10. **UTF-8 Safe String Operations** - No `.get(..n)` on strings in TUI code
11. **TUI Zero Guards** - Division by width/height must have zero checks
12. **Decision Functions in mod.rs Only** - `await_*_decision` functions only called from workflow `mod.rs`
13. **No expect() in Phase Handlers** - Workflow phase handlers must use Result types, not expect()
14. **No send().unwrap() on Channels** - Channel sends must handle dropped receivers gracefully
15. **User Channels Use select!** - approval_rx/control_rx must use tokio::select! for quit handling

### Test Location Pattern

```rust
// src/foo/bar.rs
fn private_fn() { }

#[cfg(test)]
#[path = "tests/bar.rs"]
mod tests;
```

```rust
// src/foo/tests/bar.rs
use super::*;

#[test]
fn test_private_fn() {
    private_fn();  // Works - tests is a child module
}
```

## Code Quality Standards

### No Shortcuts

- **DO NOT** dismiss failures as "flaky" without investigation
- **DO NOT** create wrapper functions to avoid refactoring
- **DO NOT** add `#[allow(...)]` to silence warnings
- **DO NOT** leave backwards compatibility code

### No Mocking

Tests use real implementations only:
- Real servers, real connections, real file systems
- No mock/fake/stub implementations
- No "TestFoo" versions of production types

### No Backwards Compatibility

- **DELETE** old code paths immediately
- **DELETE** deprecated functions (no `#[deprecated]`)
- **DELETE** legacy format support
- **DELETE** unused parameters

### Refactoring is Encouraged

Change as much code as needed to fix problems properly:
- Update all call sites when changing signatures
- Rename types/functions across the entire codebase
- Delete and rewrite broken code
- Never choose "minimal change" over correctness

### No Timelines in Plans

Plans must not include time estimates, schedules, or deadlines. Focus on technical scope and sequencing only.

## TUI Safety Patterns

### UTF-8 String Handling

**Never use byte-based operations for display truncation.** Multi-byte characters (emoji, non-ASCII) will break.

```rust
// WRONG - byte-based truncation breaks on UTF-8
if name.len() > 15 {
    format!("{}...", name.get(..12).unwrap_or(name))
}

// CORRECT - character-based truncation
if name.chars().count() > 15 {
    format!("{}...", name.chars().take(12).collect::<String>())
}
```

For cursor-based slicing (where byte positions are maintained at UTF-8 boundaries), use `tui/cursor_utils.rs`:

```rust
use crate::tui::cursor_utils::{slice_up_to_cursor, slice_from_cursor, slice_between_cursors};

// Instead of: text.get(..cursor).unwrap_or("")
let before = slice_up_to_cursor(text, cursor);

// Instead of: text.get(cursor..).unwrap_or("")
let after = slice_from_cursor(text, cursor);

// Instead of: text.get(start..end).unwrap_or("")
let middle = slice_between_cursors(text, start, end);
```

The cursor_utils helpers include `debug_assert!` to validate positions are at character boundaries.

### Terminal Safety

1. **Panic Hook**: The TUI sets a panic hook to restore terminal state on crash (disable raw mode, leave alternate screen, show cursor)

2. **Zero Guards**: Always check width/height before division:
```rust
if width == 0 || height == 0 {
    return; // Can't render in zero-size area
}
let columns = total / width;
```

3. **Bounded Buffers**: Output buffers have maximum sizes to prevent memory growth:
   - `MAX_OUTPUT_LINES = 10000`
   - `MAX_STREAMING_LINES = 5000`

4. **Control Character Sanitization**: Agent output is sanitized before display to prevent terminal corruption
