## 1. Project Scaffolding

- [x] 1.1 Set up Cargo.toml with all dependencies (rmcp, tokio, axum, rusqlite, git2, reqwest, serde, clap, etc.)
- [x] 1.2 Create module structure: main.rs, server.rs, db.rs, team.rs, task.rs, message.rs, worktree.rs, opencode.rs, event.rs, spawner/mod.rs, spawner/cmux.rs, bridge.rs, config.rs
- [x] 1.3 Implement CLI argument parsing with clap (port, db path, project path, log level) and tracing setup

## 2. Database Layer

- [x] 2.1 Implement SQLite schema initialization with all tables (teams, members, tasks, task_dependencies, messages, worktrees, file_changes, agent_events) and WAL mode
- [x] 2.2 Implement single-writer + reader pool pattern (Mutex<Connection> for writes, r2d2 pool for reads)
- [x] 2.3 Implement team CRUD queries (create, get, update status, list)
- [x] 2.4 Implement member CRUD queries (create profile, get by ID, update status, list by team)
- [x] 2.5 Implement task CRUD queries (create, get, update status/owner, list with filters: mine/all/available)
- [x] 2.6 Implement task dependency queries (add edge, get dependencies, get dependents, check for cycles, compute unblocked tasks)
- [x] 2.7 Implement message queries (insert, get by recipient/topic, mark as read, list inbox)
- [x] 2.8 Implement worktree queries (create, update status/base_commit, get by profile)
- [x] 2.9 Implement file_changes and agent_events queries (insert, query by team/agent/file)

## 3. MCP Server Core

- [x] 3.1 Implement ServerHandler struct with shared AppState (db, config, spawner, opencode clients)
- [x] 3.2 Implement dual transport setup: stdio server + axum HTTP server with streamable HTTP transport
- [x] 3.3 Implement agent identification middleware: extract caller identity from stdio (lead) or X-Agent-Profile header (workers)
- [x] 3.4 Register all MCP tool stubs with rmcp #[tool] proc macros and JSON schema parameter types

## 4. OpenSpec Bridge

- [x] 4.1 Implement tasks.md parser: parse numbered sections and checkbox items into structured task list with source_ids
- [x] 4.2 Implement tasks.md exporter: render SQLite task state back to tasks.md format with checkbox states
- [x] 4.3 Implement reimport_tasks diff logic: compare new tasks.md against current SQLite state, produce add/remove/modify diff

## 5. Team Lifecycle

- [x] 5.1 Implement team_create tool: validate inputs, create team record, call bridge import, register lead as member, return team ID
- [x] 5.2 Implement team_status tool: aggregate team, member, task, and conflict state into summary response
- [x] 5.3 Implement team_end tool: call export_tasks, kill active agents, clean up worktrees, mark team completed

## 6. Git Worktree Operations

- [x] 6.1 Implement worktree creation using git2: create worktree at .worktrees/<name>/, create branch teams/<name> from HEAD
- [x] 6.2 Implement worktree_status tool: branch state, divergence from main, modified files list using git2 + git CLI
- [x] 6.3 Implement merge_to_main tool: attempt merge via git CLI, detect conflicts, identify counterpart agents from file_changes
- [x] 6.4 Implement get_conflict_details tool: return conflicting files, both sides, counterpart agent names
- [x] 6.5 Implement get_agent_diff tool: return another agent's changes relative to common ancestor
- [x] 6.6 Implement rebase_from_main tool: rebase agent branch onto main via git CLI, handle conflicts
- [x] 6.7 Implement worktree cleanup: remove worktree directory, optionally delete branch

## 7. Task Engine

- [x] 7.1 Implement task_list tool: query tasks with mine/all/available filters, using caller context for scoping
- [x] 7.2 Implement task_set_dependency tool: add dependency edge with cycle detection, update blocked status
- [x] 7.3 Implement task_claim tool: atomic claim with BEGIN IMMEDIATE transaction, validate task is pending and unblocked
- [x] 7.4 Implement task_complete tool: mark completed, compute newly unblocked tasks, trigger event dispatcher
- [x] 7.5 Implement task_fail tool: mark failed with reason, trigger event dispatcher to notify lead

## 8. Message Bus

- [x] 8.1 Implement send_message tool: store message in SQLite, route to recipient via OpenCode SDK push
- [x] 8.2 Implement read_inbox tool: query messages for caller, optional unread_only filter, mark as read
- [x] 8.3 Implement topic routing logic: @agent-name for direct, #team for broadcast to all active agents, #conflict for conflict-related

## 9. OpenCode SDK Client

- [x] 9.1 Implement OpenCode HTTP client struct with reqwest: base URL, session management methods
- [x] 9.2 Implement spawn_serve: launch `opencode serve` as tokio Child, capture stderr, store process handle
- [x] 9.3 Implement health probe: retry loop on health endpoint with configurable timeout
- [x] 9.4 Implement create_session: POST /session with agent type, store session ID
- [x] 9.5 Implement push_prompt: POST /session/{id}/prompt to deliver messages to agent sessions
- [x] 9.6 Implement SSE event stream subscription: GET /event with reconnection logic, emit events to dispatcher
- [x] 9.7 Implement child process monitor: detect unexpected exits, trigger crash dispatcher flow
- [x] 9.8 Implement port allocator: track used ports, find available ports, release on agent kill

## 10. Event Dispatcher

- [x] 10.1 Implement task completion dispatcher: unblock dependents, push status to lead, push availability to unblocked agents, update cmux
- [x] 10.2 Implement task failure dispatcher: push failure details to lead, update cmux log
- [x] 10.3 Implement merge conflict dispatcher: identify counterpart, push negotiation context to counterpart and status to lead, update cmux
- [x] 10.4 Implement merge success dispatcher: broadcast merge notification to all active agents, update cmux
- [x] 10.5 Implement crash dispatcher: mark agent crashed, mark tasks failed, capture cmux screen, push crash report to lead, update cmux
- [x] 10.6 Implement convergence dispatcher: detect all tasks complete, push convergence message to lead

## 11. cmux Spawner

- [x] 11.1 Implement Spawner trait and CmuxSpawner struct with Unix socket JSON-RPC client
- [x] 11.2 Implement cmux socket detection: check CMUX_SOCKET_PATH env var and known default paths
- [x] 11.3 Implement create_workspace: new-workspace + rename-workspace + new-split right
- [x] 11.4 Implement send_keys: send opencode attach command to left pane
- [x] 11.5 Implement sidebar operations: set-status, set-progress, log, notify
- [x] 11.6 Implement read_screen for crash context capture
- [x] 11.7 Implement destroy_workspace for cleanup
- [x] 11.8 Implement HeadlessSpawner as no-op fallback (log-only, all trait methods return Ok)

## 12. Agent Management

- [x] 12.1 Implement agent_profile_create tool: create member, worktree, branch, write opencode.json and worker.md into worktree
- [x] 12.2 Implement agent_spawn tool: call OpenCode SDK spawn_serve, health probe, create session, push initial prompt, subscribe SSE, call cmux spawner
- [x] 12.3 Implement agent_whoami tool: return profile, tasks, context from caller identity
- [x] 12.4 Implement agent_status tool: return health, current task, progress, worktree state
- [x] 12.5 Implement agent_kill tool: terminate process, preserve branch, mark tasks failed, record context, update cmux

## 13. Integration and Testing

- [x] 13.1 Write integration test: team_create imports tasks from a sample tasks.md and verifies SQLite state
- [x] 13.2 Write integration test: task dependency DAG with claim/complete/unblock cycle
- [x] 13.3 Write integration test: message routing for direct and topic messages
- [x] 13.4 Write integration test: worktree creation, branch management, and cleanup
- [x] 13.5 Write integration test: dual transport startup (stdio + HTTP) with agent identification
- [x] 13.6 End-to-end test: full lifecycle with mock OpenCode SDK (create team, spawn agents, complete tasks, merge, end)
