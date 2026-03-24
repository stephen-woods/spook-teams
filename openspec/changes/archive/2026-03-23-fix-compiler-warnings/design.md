## Context

The spook-teams project is a Rust application that accumulates compiler warnings over time as code evolves -- imports become stale, variables go unused after refactors, and prototyped code (like bridge diff structs and spawner methods) remains after direction changes. Currently `cargo build` produces 16+ unique warnings across `src/event.rs`, `src/opencode.rs`, `src/server.rs`, `src/worktree.rs`, `src/agent.rs`, `src/task.rs`, `src/main.rs`, `src/bridge.rs`, `src/spawner/mod.rs`, and `src/db/file_changes.rs`.

## Goals / Non-Goals

**Goals:**
- Eliminate all compiler warnings so `cargo build` produces zero warnings
- Keep changes minimal and mechanical -- no behavioral changes

**Non-Goals:**
- Enabling `#[deny(warnings)]` project-wide (can be considered separately)
- Refactoring or restructuring any module beyond what's needed to fix warnings
- Restoring functionality for removed dead code -- if the compiler says it's dead, it goes

## Decisions

### 1. Remove dead code rather than suppress with `#[allow(unused)]`

Suppressing warnings hides real issues. Since every flagged item is genuinely unused (verified by the compiler across both lib and bin targets), removing the code is cleaner. If any of this code is needed later, git history preserves it.

**Alternative considered**: Adding `#[allow(dead_code)]` attributes. Rejected because it masks the symptom without addressing the root cause.

### 2. Prefix unused variables with `_` rather than removing them

For destructuring patterns (e.g., `let (member, project_path) = ...`), prefixing with `_` is the idiomatic Rust approach. It documents that the value is intentionally ignored while keeping the destructuring shape intact.

### 3. Handle unused struct fields case-by-case

- `tool_router` in `SpookTeamsHandler`: Prefix with `_` if the field may be needed soon, otherwise remove.
- `summary` in `TaskCompleteParams`: Prefix with `_` since it's a deserialized API field that may be used later.
- `port`/`worktree` in `ManagedProcess`: Prefix with `_` since they're stored at construction but not yet read.

**Rationale**: Struct fields used in serialization/deserialization or stored for future use should be prefixed rather than removed to preserve the data model.

### 4. Remove the `destroy_workspace` trait method from `Spawner`

This is an unused trait method. Removing unused trait methods is safe since no implementor needs to provide it and no caller uses it.

### 5. Remove bridge diff code entirely

`TaskDiff`, `RemovedTask`, `ModifiedTask`, `diff_reimport`, and `apply_diff` form a cohesive unit of dead code in `src/bridge.rs`. Remove all of it as a unit.

## Risks / Trade-offs

- **[Risk] Removing code that's planned for future use** → Mitigation: Git history preserves everything. Struct fields that look intentionally stored are prefixed with `_` instead of removed.
- **[Risk] Removing a trait method breaks downstream implementors** → Mitigation: `destroy_workspace` has no callers and no implementations use it. The project is self-contained.
- **[Risk] Removing `pub` items that external consumers depend on** → Mitigation: This is an application binary, not a library crate consumed by others. All `pub` items are internal.
