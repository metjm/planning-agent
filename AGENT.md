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

## General Principles

1. **Make changes fully** - Don't leave partial migrations or TODO comments for "later"
2. **Delete unused code** - If something isn't called, delete it
3. **No dead code behind `#[allow(dead_code)]`** - Fix the issue or delete the code
4. **Clean up warnings** - Don't suppress warnings, fix the underlying issue
