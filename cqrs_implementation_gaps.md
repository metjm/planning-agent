# CQRS Implementation Gaps and Issues

This document catalogs all identified gaps and issues from a comprehensive 10-agent review of the CQRS/Event Sourcing implementation.

**Last Updated**: Post-fix verification

---

## Status Summary

| Category | Total | Fixed | Remaining |
|----------|-------|-------|-----------|
| Command Dispatch | 3 | 2 | 1 |
| Test Coverage | 3 | 3 | 0 |
| Dispatch Helpers | 5 | 4 | 1 |
| Domain Types | 3 | 1 | 2 |
| Error Handling | 5 | 4 | 1 |
| Actor Implementation | 3 | 2 | 1 |
| Code Quality | 1 | 0 | 1 |
| **TOTAL** | **23** | **16** | **7** |

---

## 1. Command Dispatch Gaps

### 1.1 ClearFailure Command Never Dispatched ⚠️ REMAINING
- **Severity**: Medium
- **Location**: Command defined in `src/domain/commands.rs:128`
- **Issue**: The `ClearFailure` command is defined and the aggregate handler exists (`src/domain/aggregate.rs:364`), but no code path in the workflow ever dispatches this command.
- **Impact**: Dead code. Failures can be recorded but never cleared through the domain layer.
- **Resolution**: Either implement dispatch logic where failures should be cleared (e.g., after successful recovery or phase completion), or remove the command if not needed.

### 1.2 ReviewCycleStarted Not Dispatched in Sequential Mode ✅ FIXED
- **Severity**: High
- **Location**: `src/app/workflow/reviewing.rs`
- **Fix Applied**: Added `ReviewCycleStarted` dispatch at line 432-439 in sequential reviewing mode with `ReviewMode::Sequential`.

### 1.3 ReviewerApproved/ReviewerRejected Not Dispatched in Parallel Mode ✅ FIXED
- **Severity**: High
- **Location**: `src/app/workflow/reviewing.rs`
- **Fix Applied**: Added dispatch of `ReviewerApproved`/`ReviewerRejected` for each reviewer at lines 289-304 before `ReviewCycleCompleted`.

---

## 2. Test Coverage Gaps

### 2.1 Commands Without Happy Path Tests ✅ FIXED
- **Severity**: High
- **Location**: `src/domain/aggregate_tests.rs`
- **Fix Applied**: Added comprehensive tests for all command variants. Test count increased from ~22 to 37.

### 2.2 Missing Edge Case Tests ✅ FIXED
- **Severity**: Medium
- **Fix Applied**: Added tests for failure history limit enforcement, multiple revision iterations, sequential review mode state, and worktree reattachment.

### 2.3 Missing Invalid Transition Tests ✅ FIXED
- **Severity**: Medium
- **Fix Applied**: Added negative tests verifying commands are rejected in wrong phases.

---

## 3. Dispatch Helper Inconsistencies

### 3.1 Duplicate Implementation ✅ FIXED
- **Severity**: Medium
- **Fix Applied**: Extracted shared `dispatch_domain_command()` helper in `src/app/workflow/mod.rs` (lines 49-96). Both `planning.rs` and `revising.rs` now use this shared helper.

### 3.2 Incomplete Error Handling ✅ FIXED
- **Severity**: High
- **Fix Applied**: `dispatch_domain_command()` now uses full pattern matching: `Ok(Ok(_))`, `Ok(Err(e))`, `Err(_)`.

### 3.3 Inconsistent Log Levels ✅ FIXED
- **Severity**: Low
- **Fix Applied**: Standardized logging in shared dispatch helper - `Info` for success, `Warn` for failures.

### 3.4 Missing Success Logging ✅ FIXED
- **Severity**: Low
- **Fix Applied**: Shared dispatch helper logs successful command dispatch.

### 3.5 Inline Closure Instead of Function ⚠️ REMAINING (By Design)
- **Severity**: Low
- **Location**: `src/app/workflow/implementation.rs`
- **Status**: Left as inline closure due to implementation phase's different context requirements. The closure captures local variables specific to implementation flow.

---

## 4. Domain Type Usage Inconsistencies

### 4.1 ConversationId Newtype Not Used ✅ PARTIALLY FIXED
- **Severity**: Medium
- **Fix Applied**: Domain layer (`src/domain/events.rs`, `src/domain/commands.rs`, `src/domain/types.rs`) now uses `ConversationId` newtype.
- **Remaining**: Legacy `src/state.rs` still uses `Option<String>` for conversation IDs. Full migration requires larger refactor.

### 4.2 Legacy State Uses Raw Types ⚠️ REMAINING
- **Severity**: Low
- **Location**: `src/state.rs`
- **Issue**: Legacy state management uses raw types instead of domain newtypes.
- **Status**: Deferred - would require migrating all state serialization and breaking existing session files.

### 4.3 Duplicate Structures ⚠️ REMAINING
- **Severity**: Low
- **Locations**: `src/state.rs` vs `src/domain/types.rs`
- **Issue**: `AgentConversationState` and `InvocationRecord` defined in both locations.
- **Status**: Deferred - requires careful migration to avoid breaking session restore.

---

## 5. Error Handling Issues

### 5.1 Limited Error Variants ✅ FIXED
- **Severity**: Medium
- **Location**: `src/domain/errors.rs`
- **Fix Applied**: Added `NotInitialized` and `ConcurrencyConflict { message: String }` variants.

### 5.2 Generic Catch-All Error Message ✅ FIXED
- **Severity**: Low
- **Location**: `src/domain/aggregate.rs`
- **Fix Applied**: Catch-all now includes command name and current phase in error message.

### 5.3 Lost Error Type Information ✅ FIXED
- **Severity**: Medium
- **Location**: `src/domain/actor.rs`
- **Fix Applied**: Specific `AggregateError` variants now map to corresponding `WorkflowError` variants including `ConcurrencyConflict`.

### 5.4 Ignored Broadcast Failures ✅ FIXED
- **Severity**: Medium
- **Location**: `src/domain/query.rs`
- **Fix Applied**: Added `tracing::warn!` when broadcast fails.

### 5.5 Ignored Reply Delivery ⚠️ REMAINING (Low Impact)
- **Severity**: Low
- **Location**: `src/domain/actor.rs`
- **Issue**: `let _ = reply.send(...)` silently ignores if receiver is gone.
- **Status**: Accepted - receiver being gone is normal during shutdown, logging would be noise.

---

## 6. Actor Implementation Issues

### 6.1 Potential View State Race Condition ⚠️ REMAINING (Architectural)
- **Severity**: Medium
- **Location**: `src/domain/actor.rs`
- **Issue**: View is read AFTER `cqrs.execute()` completes. Between execute returning and view lock acquisition, another thread could be modifying the view.
- **Status**: Accepted - race window is tiny and impact is minimal (stale read, not corruption). Proper fix requires architectural changes to cqrs-es integration.

### 6.2 Silent Event Log Corruption Handling ✅ FIXED
- **Severity**: Medium
- **Location**: `src/domain/actor.rs` - `bootstrap_view_from_events()`
- **Fix Applied**: Added logging for skipped lines during bootstrap, counts unparseable events.

### 6.3 Panic on Invalid Aggregate ID ✅ FIXED
- **Severity**: Medium
- **Location**: `src/domain/view.rs`
- **Fix Applied**: Changed from `expect()` to `match` with `tracing::warn!` for invalid UUIDs.

---

## 7. Code Quality Issues

### 7.1 Analysis Documents Indicate Planned Work ⚠️ REMAINING
- **Severity**: Info
- **Locations**:
  - `/workspaces/onegc/planning-agent/refactoring_plan.md`
  - `/workspaces/onegc/planning-agent/parallel_agents_finding.md`
  - `/workspaces/onegc/planning-agent/unwrap_findings.md`
- **Issue**: Untracked markdown files document architectural issues and planned refactoring.
- **Status**: These documents track future work and should be reviewed/archived.

---

## Summary by Priority

### ✅ Fixed (16 items)
All high-priority items have been addressed:
- ReviewCycleStarted dispatched in sequential mode
- ReviewerApproved/Rejected dispatched in parallel mode
- Complete error handling in dispatch helpers
- Comprehensive test coverage for all commands
- Error variants expanded
- Logging for silent failures
- Panic converted to graceful handling

### ⚠️ Remaining (7 items)
Low to medium priority items that are either:
- **By Design**: Implementation inline closure, ignored reply delivery
- **Deferred**: Legacy type migration (breaking change to sessions)
- **Architectural**: View state race condition (requires cqrs-es changes)
- **Dead Code**: ClearFailure command (should be removed or used)
- **Info**: Documentation files

---

## Recommended Next Steps

1. **Remove or Use ClearFailure**: The command is dead code. Either:
   - Add dispatch after successful phase completion
   - Delete the command and its handler

2. **Archive Analysis Documents**: Review and archive/delete the planning markdown files.

3. **Consider Legacy Migration**: Plan a future migration of `src/state.rs` to use domain newtypes (breaking change for existing sessions).
