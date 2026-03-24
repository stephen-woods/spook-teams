## Why

The project currently generates 16+ compiler warnings across 7 source files, covering unused imports, unused variables, dead code (unused structs, functions, methods, and fields), and an unused enum variant. These warnings add noise to build output, obscure real issues, and signal incomplete cleanup from prior refactors. Fixing them now keeps the codebase clean and prevents warning accumulation.

## What Changes

- Remove unused `use` imports in `src/event.rs`, `src/opencode.rs`, `src/server.rs`, `src/worktree.rs`, `src/main.rs`, and `src/spawner/mod.rs`
- Prefix unused variables with `_` in `src/agent.rs`, `src/event.rs`, `src/worktree.rs`, and `src/task.rs`
- Remove or prefix unused struct fields (`tool_router` in `SpookTeamsHandler`, `summary` in `TaskCompleteParams`, `port`/`worktree` in `ManagedProcess`)
- Remove dead code: unused methods (`resolve_task_id`, `with_caller`, `pid`, `is_lead`, `register_client`, `register_workspace`, `remove_client`, `destroy_workspace`), unused structs (`TaskDiff`, `RemovedTask`, `ModifiedTask`), unused functions (`get_latest_event`, `diff_reimport`, `apply_diff`), and unused enum variant (`SplitDirection::Down`)
- Remove unused re-export `CmuxSpawner` from `src/spawner/mod.rs`

## Capabilities

### New Capabilities

_None_ -- this is a cleanup change with no new capabilities.

### Modified Capabilities

_None_ -- no spec-level behavior changes. All removals target verified dead code.

## Impact

- **Code**: `src/event.rs`, `src/opencode.rs`, `src/server.rs`, `src/worktree.rs`, `src/agent.rs`, `src/task.rs`, `src/main.rs`, `src/bridge.rs`, `src/spawner/mod.rs`, `src/db/file_changes.rs`
- **APIs**: No public API changes -- all removed items are internal/unused
- **Dependencies**: No dependency changes
- **Risk**: Low -- removing only verified dead code and unused imports; the compiler itself identified every item
