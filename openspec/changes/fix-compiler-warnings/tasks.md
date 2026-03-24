## 1. Remove Unused Imports

- [x] 1.1 Remove unused `Deserialize`, `Serialize` imports and unused `info` import in `src/event.rs`
- [x] 1.2 Remove unused `HashMap` and `Arc` imports in `src/opencode.rs`
- [x] 1.3 Remove unused `axum::Router` and unused `Serialize` imports in `src/server.rs`
- [x] 1.4 Remove unused `member::MemberStatus` import in `src/worktree.rs`
- [x] 1.5 Remove unused `std::path::PathBuf` import in `src/main.rs`
- [x] 1.6 Remove unused `pub use cmux::CmuxSpawner` re-export in `src/spawner/mod.rs`

## 2. Fix Unused Variables

- [x] 2.1 Prefix unused `project_path` with `_` in `src/agent.rs:425`
- [x] 2.2 Prefix unused `lead_member_id` with `_` in `src/event.rs:98`
- [x] 2.3 Prefix unused `clients` with `_` in `src/event.rs:124`
- [x] 2.4 Prefix unused `agent_name` with `_` in `src/worktree.rs:296`
- [x] 2.5 Prefix unused `project_path` with `_` in `src/worktree.rs:412`
- [x] 2.6 Prefix unused `team_id` with `_` in `src/task.rs:359`

## 3. Remove Dead Code in `src/server.rs`

- [x] 3.1 Prefix unused `tool_router` field with `_` in `SpookTeamsHandler` struct
- [x] 3.2 Remove unused method `resolve_task_id` from `SpookTeamsHandler` impl
- [x] 3.3 Remove unused method `with_caller` from `SpookTeamsHandler` impl
- [x] 3.4 Remove unused method `is_lead` from `CallerContext` impl
- [x] 3.5 Prefix unused `summary` field with `_` in `TaskCompleteParams` struct

## 4. Remove Dead Code in `src/opencode.rs`

- [x] 4.1 Prefix unused `port` and `worktree` fields with `_` in `ManagedProcess` struct
- [x] 4.2 Remove unused method `pid` from `ManagedProcess` impl

## 5. Remove Dead Code in `src/event.rs`

- [x] 5.1 Remove unused methods `register_client`, `register_workspace`, and `remove_client` from `EventDispatcher` impl

## 6. Remove Dead Code in `src/bridge.rs`

- [x] 6.1 Remove unused structs `TaskDiff`, `RemovedTask`, and `ModifiedTask`
- [x] 6.2 Remove unused functions `diff_reimport` and `apply_diff`

## 7. Remove Dead Code in Other Files

- [x] 7.1 Remove unused function `get_latest_event` in `src/db/file_changes.rs`
- [x] 7.2 Remove unused enum variant `SplitDirection::Down` in `src/spawner/mod.rs`
- [x] 7.3 Remove unused trait method `destroy_workspace` from `Spawner` trait in `src/spawner/mod.rs`

## 8. Verify

- [x] 8.1 Run `cargo build` and confirm zero warnings
- [x] 8.2 Run `cargo test` and confirm all tests pass
