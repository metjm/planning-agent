# Architectural Refactoring Plan for `planning-agent`

## 1. Executive Summary

### 1.1. The Problem

Our current architecture has a significant disconnect between its documented design and its actual implementation. The codebase contains a well-defined `WorkflowStateMachine` intended to be the central authority for state transitions, but it is entirely bypassed by the main application. Instead, the core workflow logic, primarily in `src/app/workflow/mod.rs`, directly manipulates a mutable `State` object.

This procedural, ad-hoc state management leads to several problems:
- **Brittleness**: State transitions are not explicitly defined or enforced, making it easy to get the application into an invalid state.
- **Low Visibility**: There is no single source of truth for the application's state, and no history of how we arrived at the current state.
- **Poor Extensibility**: Adding new phases or features requires modifying complex, procedural code, increasing the risk of introducing bugs.
- **Difficult Maintenance**: The codebase is hard to reason about, leading to a high maintenance burden and slowing down future development.

### 1.2. The Solution

This plan proposes a total refactor to embrace an **Event Sourcing (ES) and CQRS** architecture. This is the most robust pattern for achieving our goals of rock-solid stability, full visibility, and maximum extensibility.

**Core Idea**: Instead of storing the current state, we store an immutable, append-only log of every **Event** that has ever occurred (e.g., `PlanningCompleted`, `ReviewerApprovedPlan`). The current state is a **projection**—a derived view rebuilt by replaying this event log.

This architecture is powered by:
- **`cqrs-es`**: A mature Rust library for implementing Event Sourcing and CQRS.
- **`ractor`**: A lightweight, `tokio`-native actor framework with supervision for resilience.

## 2. Core Architectural Principles

- **Events as the Source of Truth**: The append-only event log is the canonical record of everything that has happened. State is always derivable from this log.
- **Commands Express Intent**: External components (TUI, Host, Daemon) send **Commands** (e.g., `SubmitPlan`, `ApproveReview`) to express their *intent* to change the system. Commands can be rejected.
- **Events Record Facts**: When a command is successfully processed, it produces one or more **Events** (e.g., `PlanSubmitted`, `ReviewApproved`). Events are immutable facts that have already happened; they cannot be rejected.
- **State is a Projection**: The current `WorkflowState` is built by applying all events in sequence. This "read model" can be rebuilt at any time from the event log.
- **Full State Visibility**: Any part of the system can subscribe to the event stream to build its own view of the state. The TUI, Host, and Daemon can all have tailored projections optimized for their needs.
- **Decoupled Components**: The UI and other services are decoupled from business logic. They dispatch commands and subscribe to state projections.

## 3. The New Architecture

### 3.1. Key Components

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Event Store                                    │
│  (Append-only log of all Events, persisted to disk)                         │
└─────────────────────────────────────────────────────────────────────────────┘
        ▲                                           │
        │ Persist                                   │ Replay / Stream
        │                                           ▼
┌───────┴───────┐       ┌───────────────────────────────────────────────────┐
│   Aggregate   │◄──────│              Command Handler                      │
│  (Domain      │       │  (Validates commands against current state,       │
│   Logic)      │       │   produces Events)                                │
└───────────────┘       └───────────────────────────────────────────────────┘
        ▲                                           │
        │ Query State                               │ Emit Events
        │                                           ▼
┌───────┴───────────────────────────────────────────────────────────────────┐
│                           Event Bus / Projector                           │
│  (Fans out events to all subscribers)                                     │
└───────────────────────────────────────────────────────────────────────────┘
        │                       │                       │
        ▼                       ▼                       ▼
┌───────────────┐       ┌───────────────┐       ┌───────────────┐
│  TUI View     │       │  Host View    │       │  Daemon View  │
│  (Projection) │       │  (Projection) │       │  (Projection) │
└───────────────┘       └───────────────┘       └───────────────┘
```

### 3.2. Domain Model (using `cqrs-es` concepts)

**The Aggregate: `WorkflowAggregate`**

The aggregate is the domain object that enforces all business rules. It holds the current state and decides which commands are valid.

```rust
// src/domain/aggregate.rs
use cqrs_es::Aggregate;

#[derive(Default, Serialize, Deserialize)]
pub struct WorkflowAggregate {
    pub phase: Phase,
    pub iteration: u32,
    pub plan_path: Option<PathBuf>,
    // ... other state fields
}

#[async_trait]
impl Aggregate for WorkflowAggregate {
    type Command = WorkflowCommand;
    type Event = WorkflowEvent;
    type Error = WorkflowError;
    type Services = WorkflowServices; // For side effects like calling agents

    fn aggregate_type() -> String {
        "workflow".to_string()
    }

    async fn handle(
        &self,
        command: Self::Command,
        services: &Self::Services,
    ) -> Result<Vec<Self::Event>, Self::Error> {
        // Validate command against current state, return events or error
        match command {
            WorkflowCommand::StartPlanning { objective } => {
                if self.phase != Phase::Initial {
                    return Err(WorkflowError::InvalidTransition);
                }
                Ok(vec![WorkflowEvent::PlanningStarted { objective }])
            }
            WorkflowCommand::SubmitPlan { plan_path } => {
                if self.phase != Phase::Planning {
                    return Err(WorkflowError::InvalidTransition);
                }
                Ok(vec![WorkflowEvent::PlanSubmitted { plan_path }])
            }
            // ... other commands
        }
    }

    fn apply(&mut self, event: Self::Event) {
        // Mutate state based on event (this is the projection logic)
        match event {
            WorkflowEvent::PlanningStarted { .. } => {
                self.phase = Phase::Planning;
            }
            WorkflowEvent::PlanSubmitted { plan_path } => {
                self.phase = Phase::Reviewing;
                self.plan_path = Some(plan_path);
            }
            // ... other events
        }
    }
}
```

**Commands**

Commands represent user/system intent. They can fail validation.

```rust
// src/domain/commands.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowCommand {
    StartPlanning { objective: String },
    SubmitPlan { plan_path: PathBuf },
    ApproveReview,
    RejectReview { feedback_path: PathBuf },
    SubmitRevision { plan_path: PathBuf },
    UserApprove,
    UserDecline,
}
```

**Events**

Events are immutable facts. They are persisted forever.

```rust
// src/domain/events.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowEvent {
    PlanningStarted { objective: String, started_at: DateTime<Utc> },
    PlanSubmitted { plan_path: PathBuf, submitted_at: DateTime<Utc> },
    ReviewStarted { reviewer: String },
    ReviewApproved { approved_at: DateTime<Utc> },
    ReviewRejected { feedback_path: PathBuf, rejected_at: DateTime<Utc> },
    RevisionStarted { iteration: u32 },
    RevisionSubmitted { plan_path: PathBuf },
    WorkflowCompleted { final_plan_path: PathBuf, completed_at: DateTime<Utc> },
    WorkflowAborted { reason: String, aborted_at: DateTime<Utc> },
}
```

### 3.3. The Actor Layer (`ractor`)

The `cqrs-es` framework handles command processing and event persistence. We wrap this in a `ractor` actor to gain:

1.  **Message-based Concurrency**: Clean async command dispatch via actor messages.
2.  **Supervision**: If the actor panics, `ractor`'s supervisor can restart it. The new actor simply replays the event log to rebuild its state—a key benefit of Event Sourcing.
3.  **Lifecycle Management**: Clean startup/shutdown semantics.

```rust
// src/domain/actor.rs
use ractor::{Actor, ActorProcessingErr, ActorRef};

pub struct WorkflowActor {
    cqrs: CqrsFramework<WorkflowAggregate, MemStore<WorkflowAggregate>>,
    event_tx: broadcast::Sender<WorkflowEvent>,
}

pub enum WorkflowMessage {
    Command(WorkflowCommand),
    GetState(oneshot::Sender<WorkflowAggregate>),
}

#[async_trait]
impl Actor for WorkflowActor {
    type Msg = WorkflowMessage;
    type State = ();
    type Arguments = WorkflowActorArgs;

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        _state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            WorkflowMessage::Command(cmd) => {
                // cqrs.execute dispatches command, persists events
                let events = self.cqrs.execute("session-id", cmd).await?;
                for event in events {
                    let _ = self.event_tx.send(event);
                }
            }
            WorkflowMessage::GetState(reply) => {
                let state = self.cqrs.query("session-id").await?;
                let _ = reply.send(state);
            }
        }
        Ok(())
    }
}
```

### 3.4. Data Flow

1.  **User presses "Approve"** in the TUI.
2.  TUI sends `WorkflowMessage::Command(WorkflowCommand::UserApprove)` to the `WorkflowActor`.
3.  The actor's `cqrs.execute()` method:
    a.  Loads current aggregate state from the event store.
    b.  Calls `aggregate.handle(command)` to validate and produce events.
    c.  Persists the new events to the event store.
    d.  Calls `aggregate.apply(event)` for each event to update in-memory state.
4.  The actor broadcasts the new events via `event_tx`.
5.  All subscribers (TUI, Host, Daemon) receive the events and update their projections.
6.  The TUI re-renders based on its updated view.

## 4. Phased Refactoring Steps

### Phase 1: Define the Domain Model

1.  **Create `src/domain/` module** with `mod.rs`, `aggregate.rs`, `commands.rs`, `events.rs`, `errors.rs`.
2.  **Define `WorkflowCommand` enum** covering all possible user/system intents.
3.  **Define `WorkflowEvent` enum** covering all state-changing facts.
4.  **Implement `WorkflowAggregate`**:
    -   Implement `cqrs_es::Aggregate` trait.
    -   `handle()` method contains all command validation logic.
    -   `apply()` method contains all state mutation logic.
5.  **Write exhaustive unit tests** for the aggregate, testing every valid and invalid command/state combination.

### Phase 2: Integrate `cqrs-es` Framework

1.  **Add dependencies**: `cqrs-es`, `async-trait`, `serde`.
2.  **Choose an Event Store**:
    -   Start with `MemStore` for development.
    -   Implement a file-based `EventStore` that persists to `~/.planning-agent/sessions/<id>/events.jsonl`.
3.  **Create `CqrsFramework` instance** configured with our aggregate and event store.
4.  **Write integration tests** that exercise the full command→event→state flow.

### Phase 3: Add the Actor Layer

1.  **Add `ractor` dependency**.
2.  **Implement `WorkflowActor`** wrapping the CQRS framework.
3.  **Define `WorkflowMessage` enum** for actor communication.
4.  **Add supervision**: Configure a supervisor to restart the actor on panic.
5.  **Instantiate the actor** in `session_daemon` for each session.

### Phase 4: Refactor Workflow Engine

1.  **Refactor `src/app/workflow/mod.rs`**:
    -   Remove `&mut State` parameter from all functions.
    -   Replace direct state mutations with command dispatches to the actor.
    -   Subscribe to the event stream to know when phases complete.
2.  **Remove `WorkflowStateMachine`**: Delete the old, unused state machine code.
3.  **Remove `state_mut()`**: This backdoor is no longer possible.

### Phase 5: Refactor TUI

1.  **Subscribe to event stream** in `src/app/tui_runner/mod.rs`.
2.  **Build a TUI-specific projection** from events (optimized for rendering).
3.  **Remove duplicated state** from TUI components.
4.  **Rewrite input handlers** to dispatch commands to the actor.

### Phase 6: Refactor Host/Daemon RPC

1.  **Command-based RPC**: RPC methods accept command payloads and forward to the actor.
2.  **Event streaming RPC**: Expose an endpoint for clients to subscribe to the event stream.
3.  **State query RPC**: Expose an endpoint to query current aggregate state.

## 5. Event Store Design

The event store is critical for durability and visibility.

### 5.1. File-Based Event Store

```
~/.planning-agent/sessions/<session-id>/
├── events.jsonl          # Append-only event log (one JSON object per line)
├── snapshot.json         # Optional: periodic state snapshot for faster replay
└── ...
```

**`events.jsonl` format**:
```json
{"sequence": 1, "event": {"PlanningStarted": {"objective": "...", "started_at": "2024-..."}}}
{"sequence": 2, "event": {"PlanSubmitted": {"plan_path": "/path/to/plan.md", "submitted_at": "..."}}}
```

### 5.2. Snapshots (Optional Optimization)

For long-running sessions with many events, periodically snapshot the aggregate state. On startup, load the snapshot and replay only events after the snapshot's sequence number.

## 6. Benefits of This Architecture

| Requirement | How ES/CQRS Delivers |
|-------------|---------------------|
| **Rock-Solid Stability** | All transitions validated in `handle()`. Invalid commands rejected. Actor supervision auto-restarts on panic. State rebuilt from durable event log. |
| **Full Visibility** | The event log is a perfect, immutable audit trail. Every state change is recorded with timestamps. Time-travel debugging is trivial. |
| **Clean State Access** | Any component subscribes to events and builds its own projection. TUI, Host, Daemon all have tailored views. |
| **Extensibility** | Adding a new phase: (1) Add new commands/events, (2) Update `handle()` and `apply()`, (3) Done. Core architecture unchanged. |
| **Testability** | Aggregates are pure functions: given state + command → events. No mocks needed. Test every scenario with simple unit tests. |

## 7. Migration Strategy

1.  **Build new system in parallel**: New code lives in `src/domain/`. Old code untouched initially.
2.  **Feature flag**: Add a config option to use new vs old workflow engine.
3.  **Gradual migration**: Port one phase at a time, validating behavior matches.
4.  **Delete old code**: Once all phases migrated and tested, remove old `WorkflowStateMachine` and procedural code.

## 8. Dependencies to Add

```toml
[dependencies]
cqrs-es = "0.4"
async-trait = "0.1"
ractor = "0.12"
```

## 9. Open Questions

1.  **Event schema evolution**: How do we handle adding new fields to events? (Recommendation: use `#[serde(default)]` for backwards compatibility)
2.  **Multi-session support**: Should the actor manage one session or multiple? (Recommendation: one actor per session for isolation)
3.  **Event retention policy**: Do we keep events forever or prune old sessions? (Recommendation: keep forever for sessions, prune on explicit user delete)
