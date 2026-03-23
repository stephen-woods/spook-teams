## Why

AI coding agents working on non-trivial changes frequently encounter tasks that could be parallelized but are forced into sequential execution. There is no production-ready MCP server that enables multiple independent OpenCode agents to work as a coordinated team — each in isolated git worktrees, communicating peer-to-peer, and merging autonomously. Existing solutions (Claude Code's experimental teams, claude-code-teams-mcp) lack worktree isolation, proper conflict resolution, reactive architectures, and OpenCode/OpenSpec integration.

## What Changes

- Build a Rust MCP server (`spook-teams`) that coordinates teams of OpenCode agents working in parallel on OpenSpec changes
- Implement dual MCP transport: stdio for the lead agent, streamable HTTP for worker agents — single process
- Create a SQLite-backed (WAL mode) coordination store for teams, agents, tasks, messages, worktrees, file changes, and events
- Implement a task engine with dependency DAG, atomic claiming, and reactive unblocking
- Build a peer-to-peer message bus with topic broadcasts (`#team`, `#conflict`) and direct messages (`@agent-name`), all routed via OpenCode SDK session push
- Implement git worktree lifecycle management (create, merge, rebase, conflict detection) using `git2` + git CLI
- Build a reactive event dispatcher that pushes state changes to agents via the OpenCode SDK — no polling anywhere
- Create a cmux spawner (behind a `Spawner` trait) for agent workspace management with sidebar status, progress bars, and notifications
- Build an OpenSpec bridge for importing `tasks.md` into SQLite and exporting back
- Write worker agent definition template and lead orchestration skill (`apply-teams`)
- Implement child process management for `opencode serve` instances with crash detection and recovery

## Capabilities

### New Capabilities
- `team-lifecycle`: Create, monitor, pause, and end agent teams from an OpenSpec change. Import tasks, manage team state, export results.
- `agent-management`: Create agent profiles, spawn OpenCode serve instances, manage health/crash detection, kill/respawn agents.
- `task-engine`: Dependency DAG, atomic task claiming, status transitions, reactive unblocking of dependent tasks.
- `message-bus`: Peer-to-peer messaging between agents via topic broadcasts and direct messages, routed through OpenCode SDK push.
- `worktree-ops`: Git worktree creation/removal, branch management, merge-to-main, rebase, conflict detection and details.
- `event-dispatcher`: Reactive core that translates MCP tool calls and agent events into cascading pushes to other agents and cmux sidebar updates.
- `cmux-spawner`: Agent workspace creation in cmux with split panes, opencode attach TUI, sidebar status/progress/logs/notifications.
- `openspec-bridge`: Import tasks.md into SQLite, export SQLite state back to tasks.md, reimport with diff/reconcile for mid-session spec changes.
- `opencode-sdk-client`: HTTP client for OpenCode serve instances — session creation, prompt injection, SSE event stream subscription, health monitoring.
- `mcp-server-core`: Dual-transport MCP server (stdio + streamable HTTP) with agent identification, shared state, and tool definitions using rmcp.

### Modified Capabilities

_None — this is a greenfield project._

## Impact

- **New binary crate**: `spook-teams` with ~12 source modules
- **Dependencies**: rmcp, tokio, axum, reqwest, rusqlite, git2, serde, clap, and supporting crates
- **External integrations**: OpenCode (serve/attach model, SDK API), cmux (JSON-RPC socket), git (CLI + libgit2), OpenSpec (tasks.md format)
- **New OpenCode artifacts**: `apply-teams` skill for the lead agent, `worker.md` agent definition template for workers
- **File system**: Creates git worktrees, SQLite databases, and OpenCode config files in worktree directories during operation
