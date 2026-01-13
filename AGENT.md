# Planning Agent Development Guidelines

Guidelines for AI agents (and humans) working on this codebase.

## Code Deletion Policy

**Delete deprecated code immediately. Do not keep it around for "backward compatibility".**

When refactoring or replacing functionality:

1. **Delete the old code** - Don't keep deprecated functions, types, or modules "for backward compatibility"
2. **Update all callers** - Find and update every usage of the old API to use the new one
3. **No conversion shims** - Don't add `to_new_type()` or `from_old_type()` methods just to bridge old and new code
4. **No re-exports of deprecated items** - If something is replaced, remove it entirely

### Why?

- Backward compatibility shims add complexity without value in a single-codebase project
- Keeping old code around creates confusion about which API to use
- It increases maintenance burden and test surface
- It signals uncertainty about the refactoring

### Example: Wrong

```rust
// OLD - kept around "for backward compatibility"
pub enum OldEvent { ... }
impl OldEvent {
    pub fn to_new_event(self) -> NewEvent { ... }
}

// NEW
pub enum NewEvent { ... }
```

### Example: Right

```rust
// Just the new code - old code deleted entirely
pub enum NewEvent { ... }
```

## File Size Limits

**This project enforces a 750-line maximum per file via `build.rs`.**

When a file exceeds 750 lines:
1. **Split into modules** - Extract cohesive functionality into submodules
2. **Don't compress code** - Never use hacky tricks to reduce line count (removing blank lines, combining statements, etc.)
3. **Follow existing patterns** - See `src/tui/ui/` and `src/tui/session/` for examples of split modules

### How to Split a File

When `src/foo/bar.rs` exceeds the limit:
1. Create `src/foo/bar/mod.rs` (rename the original file)
2. Extract related functions into `src/foo/bar/helpers.rs`, `src/foo/bar/types.rs`, etc.
3. Re-export public items from `mod.rs` to maintain the same external API
4. Each new file should have a clear, focused responsibility

See `docs/plans/file-line-limit.md` for detailed guidance.

## General Principles

1. **Make changes fully** - Don't leave partial migrations or TODO comments for "later"
2. **Delete unused code** - If something isn't called, delete it
3. **No dead code behind `#[allow(dead_code)]`** - Fix the issue or delete the code
4. **Clean up warnings** - Don't suppress warnings, fix the underlying issue
