## Context

spook-teams is a greenfield Rust MCP server that enables teams of AI coding agents to work in parallel on OpenSpec changes. There is no existing codebase — `src/main.rs` is a hello world and `Cargo.toml` has no dependencies.

The project exists because current multi-agent solutions (Claude Code teams, claude-code-teams-mcp) lack critical features: git worktree isolation, peer-to-peer conflict resolution, reactive event architectures, and OpenCode/OpenSpec integration. The design was thoroughly explored and documented in EXPLORE.md before this formal proposal.

**Key constraints:**
- OpenCode + OpenSpec only — no Claude Code compatibility
- cmux only for terminal multiplexer — behind a `Spawner` trait for future extensibility, but only cmux is implemented
- Fully reactive — no polling anywhere (except brief health probe at agent startup)
- Single Rust binary, single process, dual MCP transport
- Workers are autonomous peers, not subagents — communicate directly for conflict resolution

## Goals / Non-Goals

**Goals:**
- Enable parallel execution of OpenSpec change tasks across multiple independent OpenCode agents
- Provide git worktree isolation so agents never conflict at the filesystem level
- Implement reactive, push-based coordination — agents are notified immediately when relevant events occur
- Support peer-to-peer conflict negotiation when merge conflicts arise
- Give humans full visibility and intervention capability via cmux workspaces with interactive OpenCode TUIs
- Import tasks from OpenSpec's tasks.md, track execution in SQLite, and export results back

**Non-Goals:**
- Claude Code or other AI harness support
- tmux or other terminal multiplexer implementations (future work behind `Spawner` trait)
- Web UI or dashboard — cmux sidebar provides the monitoring interface
- Distributed deployment — single process, single machine
- Automatic conflict resolution without agent involvement — agents negotiate, humans escalate
- Task authoring — tasks come from OpenSpec; the MCP server only tracks execution

## Decisions

### 1. Single-Process Dual-Transport MCP Server (rmcp)

Use `rmcp` v1.2 (official Rust MCP SDK) with a single `ServerHandler` implementation serving both stdio and streamable HTTP transport.

- **Stdio**: Lead agent (user's OpenCode) spawns the MCP server as a local tool. The lead connects over stdin/stdout.
- **HTTP**: Worker agents connect as remote MCP clients with `X-Agent-Profile` header for identification. `axum` serves the HTTP transport.
- **Why single process**: Shared SQLite state, single event dispatcher, simpler deployment. Two transports are just two entry points to the same handler.
- **Alternative considered**: Separate lead and worker servers — rejected because it doubles state synchronization complexity.

### 2. SQLite with Single-Writer + Reader Pool

Use `rusqlite` with WAL mode, `r2d2` connection pool for concurrent readers, and a `Mutex<Connection>` for the single writer.

- **Write path**: All mutations go through the writer mutex with `BEGIN IMMEDIATE` transactions. This serializes writes but avoids lock contention — write operations are fast (microsecond-range SQLite inserts/updates).
- **Read path**: Reader pool serves concurrent `task_list`, `team_status`, `agent_whoami` queries without blocking on writes.
- **Why not a separate DB per team**: Teams share the same server process lifetime. A single DB simplifies cross-team queries (future) and avoids file handle proliferation.
- **Alternative considered**: In-memory state with file persistence — rejected because crash recovery is free with SQLite.

### 3. Git Operations: git2 + CLI Hybrid

Use `git2` (libgit2 bindings) for worktree creation/removal and branch management. Shell out to `git` CLI for merge and rebase operations.

- **Why hybrid**: libgit2's merge and rebase APIs are incomplete compared to the CLI. Creating worktrees and branches works well with the library API. Merge conflict reporting is richer from the CLI.
- **Worktree layout**: All worktrees live under `<project>/.worktrees/<agent-name>/`. Branch naming: `teams/<agent-name>`.
- **Alternative considered**: Pure git CLI — rejected because `git2` gives typed error handling and avoids shell quoting issues for branch/worktree creation.

### 4. Reactive Event Dispatcher

The event dispatcher is the central nervous system. It is NOT a separate task/thread — it's a set of async functions called synchronously from within MCP tool handlers.

- **Trigger**: An MCP tool call completes a state mutation (e.g., `task_complete` writes to SQLite).
- **Cascade**: The tool handler calls the dispatcher, which determines all side effects: push notifications to agents via OpenCode SDK, update cmux sidebar, unblock dependent tasks.
- **Why synchronous in the handler**: The tool response should include confirmation that side effects were initiated. If the dispatcher were a background task, the tool could return before notifications fire, creating race conditions.
- **Fire-and-forget for pushes**: The SDK push calls (`POST /session/{id}/prompt`) are spawned as independent tokio tasks. The dispatcher doesn't wait for delivery confirmation — OpenCode will process the message asynchronously.
- **Alternative considered**: Channel-based event bus with a dedicated consumer task — rejected because it adds latency and complicates error reporting. Direct function calls are simpler and debuggable.

### 5. OpenCode SDK Integration Model

Each worker is an `opencode serve` child process managed by the MCP server.

- **Spawn**: `opencode serve --port <port> --dir <worktree>` as a tokio `Child`.
- **Health**: Brief retry loop on the health endpoint (the only acceptable poll in the system, during startup only).
- **Session**: Create via `POST /session { agent: "worker" }`. Push initial prompt via `POST /session/{id}/prompt`.
- **Monitoring**: Subscribe to `GET /event` SSE stream per agent. Detect `session.idle`, `session.error`, and process exit.
- **TUI**: `opencode attach <url> -s <session_id>` runs in a cmux pane — purely for human observation, not used by the MCP server.
- **Alternative considered**: Running OpenCode directly in cmux panes with no programmatic control — rejected because there's no way to push messages or monitor events without the SDK.

### 6. cmux Spawner Behind Trait

The `Spawner` trait abstracts terminal multiplexer operations:

```rust
#[async_trait]
trait Spawner {
    async fn create_workspace(&self, name: &str, cwd: &Path) -> Result<WorkspaceId>;
    async fn create_split(&self, workspace: &WorkspaceId, direction: SplitDirection) -> Result<SurfaceId>;
    async fn send_keys(&self, surface: &SurfaceId, keys: &str) -> Result<()>;
    async fn set_status(&self, workspace: &WorkspaceId, status: &StatusUpdate) -> Result<()>;
    async fn set_progress(&self, workspace: &WorkspaceId, progress: f32, label: &str) -> Result<()>;
    async fn log(&self, workspace: &WorkspaceId, level: LogLevel, message: &str) -> Result<()>;
    async fn notify(&self, title: &str, body: &str) -> Result<()>;
    async fn read_screen(&self, workspace: &WorkspaceId) -> Result<String>;
    async fn destroy_workspace(&self, workspace: &WorkspaceId) -> Result<()>;
}
```

Only `CmuxSpawner` is implemented. It communicates via JSON-RPC over a Unix socket (`CMUX_SOCKET_PATH` or known default paths). A `HeadlessSpawner` (no-op for UI operations, logging only) is a future addition.

### 7. OpenSpec Bridge: Import/Export/Reimport

- **Import**: Parse tasks.md markdown (numbered sections + checkbox items). Create SQLite task records with `source_id` mapping (e.g., "1.1", "2.3").
- **Export**: Query SQLite, regenerate tasks.md with checkbox states reflecting completion status.
- **Reimport**: Diff new tasks.md against current SQLite state. Identify added/removed/modified tasks. Return the diff for lead agent review before applying changes. This supports mid-session spec modifications.
- **Format**: The bridge parses the specific OpenSpec tasks.md format — numbered top-level sections with `- [ ]` / `- [x]` checkbox items.

### 8. Agent Identification and Caller Context

- **Lead**: Identified by the stdio connection. There is exactly one stdio client — the lead.
- **Workers**: Identified by the `X-Agent-Profile` HTTP header on every MCP request. The `rmcp` handler extracts this and sets the caller context for all operations.
- **Caller-scoped operations**: `task_claim`, `task_complete`, `read_inbox`, `agent_whoami` all use the caller's identity to scope their behavior.
- **Why header-based**: Simple, stateless, compatible with MCP's HTTP transport. No need for MCP-level authentication — this is a local-only server.

## Risks / Trade-offs

- **[Single writer bottleneck]** → All SQLite writes go through one mutex. Mitigation: Write operations are simple inserts/updates that complete in microseconds. With <10 agents, contention is negligible. If it becomes a problem, the write path can be converted to a channel-based actor without changing the external API.

- **[OpenCode SDK stability]** → The serve/attach model and session API are relatively new features. Mitigation: Abstract SDK calls behind a client trait for easy mocking/replacement. Health probes and SSE reconnection logic handle transient failures.

- **[cmux dependency]** → Requires cmux to be running. Mitigation: The `Spawner` trait makes this pluggable. Detection at startup — if no cmux socket is found, the server can operate in headless mode (no UI, logging only). Headless spawner is a straightforward no-op implementation.

- **[Merge conflict complexity]** → Peer-to-peer negotiation may fail or loop. Mitigation: The MCP server tracks negotiation attempts. After N failed rounds, it escalates to the lead agent, who escalates to the human. The system never auto-resolves — it always involves agent reasoning.

- **[Agent crash during merge]** → If an agent crashes mid-merge, the worktree may be in an inconsistent state. Mitigation: Record merge state in SQLite before attempting. On crash recovery, detect partially-merged state and abort the merge before re-briefing the replacement agent.

- **[Port allocation for workers]** → Multiple `opencode serve` instances need unique ports. Mitigation: The MCP server manages port allocation starting from a configurable base port, checking availability before assignment. Ports are released when agents are killed.
