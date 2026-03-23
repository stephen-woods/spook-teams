# spook-teams: Exploration & Design Decisions

A Rust MCP server that enables independent AI coding agents to work as a team,
coordinated through OpenSpec and OpenCode.

## Vision

Create an MCP server that allows the creation of agent teams that work together
to solve a problem — similar to Claude Code's experimental agent teams feature,
but designed to work with OpenCode and OpenSpec. Agents are independent peers
(not subagents), each working in their own git worktree, communicating and
negotiating with each other to resolve conflicts.

## Reference Projects Analyzed

- **Claude Code Agent Teams** (built-in experimental feature): Lead + peer
  agents, shared filesystem, file-locked task list, mailbox messaging, no
  worktree isolation, no automatic conflict resolution.
- **claude-code-teams-mcp** (Python): Reimplements Claude Code teams as MCP
  server. Hub-and-spoke only (workers can't message each other), JSON files per
  task, tmux spawner, no git worktrees, no conflict detection.
- **OpenSpec**: Spec-driven development framework. File-based artifacts
  (proposal, specs, design, tasks.md). Tasks tracked via markdown checkboxes.
- **OpenCode**: Open-source AI coding agent. Supports local and remote MCP
  servers, skill system, client/server architecture, provider-agnostic.
- **cmux**: Native macOS terminal app built on libghostty. JSON-RPC socket API
  for programmatic workspace/pane management. Sidebar with status, progress,
  logs, notifications. Already has Claude Code teams integration.

---

## Decisions Made

### 1. Git Worktrees as Agent Workspaces

Each agent works in its own git worktree with its own branch.

- **Branch strategy**: One branch per agent (not per task). Simpler, avoids
  branch explosion.
- **Base branch**: Initially all worktrees branch from the same commit. As work
  progresses and merges into the main branch, newer agents branch from the
  lead's current HEAD.
- **Merge timing**: An agent attempts to merge when it has completed all of its
  assigned tasks.
- **Conflict handling**: On merge conflict, the agent identifies which other
  agent(s) caused the conflict and initiates direct peer negotiation. See
  "Conflict Negotiation Protocol" below.

### 2. Peer-to-Peer Agent Communication

Agents communicate directly with each other — not hub-and-spoke through a lead.

- **Pub/sub model with topics**:
  - `#team` — broadcasts to all agents
  - `#conflict` — merge/conflict notifications
  - `@agent-name` — direct messages
- **Conflict negotiation protocol**: When a merge conflict arises, the
  conflicting agent messages the counterpart directly. The counterpart reads
  both diffs, attempts to adjust its code for compatibility, and reports back.
- **Auto-resolution first**: Agents attempt to resolve conflicts themselves. If
  impossible, escalate to the human — not to a lead agent. (A lead agent has no
  more capability to resolve code conflicts than the worker agents do.)

### 3. Task Tracking: SQLite with tasks.md Projection

- **SQLite (WAL mode)** is the operational store for all coordination state
  during a team session: tasks, messages, worktrees, agent events, file changes.
- **tasks.md** is the initial input (imported at session start) and the output
  (exported when the session ends or on demand).
- **One-directional flow during a session**: SQLite → tasks.md. The MCP server
  is the sole writer to both.
- **Three phases**:
  1. **Planning**: tasks.md is the source of truth (human + AI craft it via
     OpenSpec)
  2. **Execution**: SQLite is the source of truth (imported from tasks.md,
     agents read/write via MCP)
  3. **Reconciliation**: SQLite state exported back to tasks.md
- **Mid-session spec changes**: Handled via a `reimport_tasks` operation that
  diffs new tasks.md against current SQLite state, determines which tasks are
  still valid, and reconciles. This requires lead agent analysis.

### 4. Task Dependencies as an Execution Concern

- Dependencies are **not encoded in tasks.md**. The markdown file describes
  *what* to build; the MCP server figures out *in what order*.
- The lead agent (LLM) analyzes all tasks at session start and determines the
  dependency graph.
- Dependencies are stored in SQLite and used to determine what's parallelizable.
- **Up-front analysis**: The lead analyzes all tasks and sets dependencies
  *before* spawning any workers. This prevents two agents from grabbing tasks
  that actually depend on each other.

### 5. Terminal Multiplexer: cmux with Abstraction Layer

- **Primary implementation**: cmux (JSON-RPC socket API)
- **Abstraction layer**: A `Spawner` trait that could support tmux or headless
  mode in the future, but only cmux is implemented now.
- **Per-agent layout**: Each agent gets a cmux workspace with a split pane —
  OpenCode on the left, shell on the right, both in the worktree directory.
- **Sidebar integration**: Agent status, task progress bars, log entries, and
  notifications via cmux's sidebar metadata API.
- **Detection**: Check for `CMUX_SOCKET_PATH` env var or known socket paths.
  Fall back to headless if unavailable.

### 6. Lead Agent Role: Coordinator Only

- The lead is the user's OpenCode session running the `apply-teams` skill.
- The lead does **not** take implementation tasks.
- The lead is responsible for: importing tasks, analyzing dependencies, creating
  agent profiles, spawning agents, monitoring progress, handling re-plans, and
  exporting final state.

### 7. Single-Process MCP Server with Dual Transport

- The MCP server is a single Rust process.
- **Stdio transport**: For the lead agent (OpenCode spawns it as a local MCP
  server).
- **HTTP transport**: For worker agents (each connects via `type: "remote"` in
  their worktree's `opencode.json`).
- Both transports hit the same SQLite state.
- **Agent identification**: Worker agents include their profile ID in an HTTP
  header (`X-Agent-Profile`). The lead is identified by the stdio connection.

### 8. Agent Profile Model

- The lead creates a profile for each worker agent via MCP before spawning.
- Each profile includes: name, worktree path, branch name, assigned tasks.
- The profile ID is passed to the worker's OpenCode instance via the
  `opencode.json` written into the worktree.
- Workers call `agent_whoami` on startup to retrieve their responsibilities.

### 9. Failure Recovery: Preserve and Brief

- When an agent crashes, its tasks are marked as **failed** (not reset to
  pending — preserve history).
- The agent's worktree and branch are **preserved** (partial work is not
  discarded).
- Crash context is recorded: last known task, terminal output, modified files.
- A replacement agent receives the crash context and can choose to: continue
  from the failed agent's branch, start fresh from main, or cherry-pick
  specific commits.

### 10. Merge Strategy: Agent Autonomy

- When one agent's branch merges into main, all other agents are notified via
  `#team` broadcast (files changed, commit range).
- Each agent **decides for itself** whether to rebase, merge, or ignore based on
  whether the merged changes overlap with its own work.
- The MCP server provides advisory tools (`worktree_status`, merge divergence
  info) but does not force any merge strategy.

### 11. Scope: OpenSpec + OpenCode Only

- The MCP server targets OpenCode as the AI harness and OpenSpec as the workflow
  framework.
- No Claude Code compatibility is planned for the initial implementation.
- This simplifies the design: no harness detection, no Claude Code CLI flags, no
  multi-harness spawner logic.

---

## SQLite Schema (Draft)

```sql
CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    openspec_change TEXT NOT NULL,
    status TEXT CHECK(status IN ('active','paused','completed')) NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE members (
    profile_id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id),
    name TEXT NOT NULL,
    role TEXT CHECK(role IN ('lead','worker')) NOT NULL,
    worktree_path TEXT,
    branch TEXT,
    status TEXT CHECK(status IN (
        'pending_spawn','active','idle','crashed','killed','completed'
    )) NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id),
    source_id TEXT NOT NULL,          -- e.g. "1.1", "2.3" from tasks.md
    title TEXT NOT NULL,
    description TEXT,
    status TEXT CHECK(status IN (
        'pending','blocked','in_progress','completed','failed','cancelled'
    )) NOT NULL,
    owner TEXT REFERENCES members(profile_id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    depends_on TEXT NOT NULL REFERENCES tasks(id),
    PRIMARY KEY (task_id, depends_on)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    sender TEXT NOT NULL,
    recipient TEXT,                    -- NULL = broadcast to topic
    topic TEXT,                        -- '#team', '#conflict', '@alice'
    message_type TEXT NOT NULL,        -- 'text', 'conflict_negotiation',
                                      -- 'merge_notification', 'system'
    body TEXT NOT NULL,
    read BOOLEAN DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE worktrees (
    profile_id TEXT PRIMARY KEY REFERENCES members(profile_id),
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    status TEXT CHECK(status IN (
        'active','merging','merged','conflict','cleaned_up'
    )) NOT NULL,
    base_commit TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE file_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    agent_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    task_id TEXT REFERENCES tasks(id),
    change_type TEXT CHECK(change_type IN ('added','modified','deleted')),
    timestamp TEXT NOT NULL
);
CREATE INDEX idx_file_changes_path ON file_changes(file_path);

CREATE TABLE agent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    agent_name TEXT NOT NULL,
    event_type TEXT NOT NULL,          -- 'spawned', 'crashed', 'completed',
                                      -- 'killed', 'merge_started',
                                      -- 'merge_completed', 'conflict_detected'
    task_id TEXT,
    details TEXT,                      -- JSON blob: terminal output, error, etc.
    timestamp TEXT NOT NULL
);
```

---

## MCP Tools (Draft)

### Team Lifecycle
| Tool | Description |
|------|-------------|
| `team_create` | Create team from OpenSpec change. Imports tasks, creates SQLite DB, starts HTTP listener. |
| `team_status` | Overview: agents, tasks, progress, active conflicts. |
| `team_end` | Export tasks.md, clean up worktrees, shut down. |

### Agent Profiles
| Tool | Description |
|------|-------------|
| `agent_profile_create` | Create profile: name, assigned tasks. Creates worktree, branch, writes opencode.json. Returns profile_id. |
| `agent_spawn` | Launch agent in cmux workspace with split pane. Sets sidebar status. |
| `agent_whoami` | Worker calls this on startup. Returns name, tasks, worktree path, instructions. |
| `agent_status` | Health, current task, progress, worktree state. |
| `agent_kill` | Preserve branch, mark tasks failed, record crash context. |

### Tasks
| Tool | Description |
|------|-------------|
| `task_list` | List tasks. Filter: mine, all, available (unblocked + unowned). |
| `task_set_dependency` | Record dependency edge between tasks. |
| `task_claim` | Atomic claim of a pending/unblocked task. |
| `task_complete` | Mark done, unblock dependents, update tasks.md projection. |
| `task_fail` | Mark failed with reason, notify lead. |

### Messages
| Tool | Description |
|------|-------------|
| `send_message` | Send to `@agent-name` or `#topic`. Supports text, conflict_negotiation, etc. |
| `read_inbox` | Read messages for calling agent. Optional unread_only filter. |

### Git / Worktree
| Tool | Description |
|------|-------------|
| `worktree_status` | Branch state, divergence from main, modified files. |
| `merge_to_main` | Attempt merge. Returns success or conflict details. |
| `get_conflict_details` | Conflicting files, both sides, counterpart agents. |
| `get_agent_diff` | Read another agent's changes (for conflict negotiation). |
| `rebase_from_main` | Rebase current branch onto main. |

### OpenSpec Bridge
| Tool | Description |
|------|-------------|
| `reimport_tasks` | Re-read tasks.md, diff against current state, return changes for review. |
| `export_tasks` | Write current SQLite state to tasks.md. |

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        spook-teams MCP Server                       │
│                              (Rust)                                 │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐    │
│  │ Team Mgmt   │  │ Task Engine │  │ Message Bus             │    │
│  │             │  │             │  │                         │    │
│  │ create/     │  │ CRUD +      │  │ peer-to-peer DMs       │    │
│  │ destroy     │  │ dependency  │  │ topic broadcasts       │    │
│  │ profiles    │  │ DAG + claim │  │ conflict negotiation   │    │
│  │ spawn/kill  │  │ import from │  │ merge notifications    │    │
│  │ health      │  │ tasks.md    │  │                         │    │
│  └──────┬──────┘  └──────┬──────┘  └───────────┬─────────────┘    │
│         │                │                      │                   │
│         └────────────────┼──────────────────────┘                   │
│                          │                                          │
│                   ┌──────┴──────┐                                   │
│                   │   SQLite    │                                   │
│                   │  (WAL mode) │                                   │
│                   │             │                                   │
│                   │ teams       │     ┌─────────────────────────┐  │
│                   │ members     │     │  Spawner Trait          │  │
│                   │ tasks       │     │                         │  │
│                   │ messages    │     │  ┌───────┐ ┌─────────┐ │  │
│                   │ worktrees   │     │  │ cmux  │ │headless │ │  │
│                   │ file_changes│     │  │(impl) │ │(future) │ │  │
│                   │ agent_events│     │  └───────┘ └─────────┘ │  │
│                   └─────────────┘     └─────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │               Worktree Manager (git2-rs)                     │  │
│  │  create / remove / merge / rebase / conflict-detect          │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │               OpenSpec Bridge                                │  │
│  │  import tasks.md → SQLite | export SQLite → tasks.md         │  │
│  │  reimport + diff + reconcile on spec changes                 │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
├───────────────────────┬─────────────────────────────────────────────┤
│   stdio (lead agent)  │         HTTP :PORT (worker agents)          │
└───────────┬───────────┴──────────┬──────────────────┬───────────────┘
            │                      │                  │
     ┌──────┴──────┐       ┌──────┴──────┐   ┌──────┴──────┐
     │ Lead Agent  │       │ Agent Alice │   │ Agent Bob   │
     │ (user's     │       │ (cmux ws)   │   │ (cmux ws)   │
     │  OpenCode)  │       │ (worktree)  │   │ (worktree)  │
     └─────────────┘       └─────────────┘   └─────────────┘
```

---

## Resolved Questions

### Q1: The `apply-teams` Skill — Lead Orchestration (Resolved)

The lead agent runs an `apply-teams` skill that orchestrates the entire team
session. The lead is **fully reactive** — no polling.

**Lifecycle:**

```
IMPORT → ANALYZE → PLAN → SPAWN → WAIT (reactive) → CONVERGE
```

1. **IMPORT**: Read OpenSpec change artifacts. Call `team_create` with project
   path + change name. Tasks imported into SQLite.
2. **ANALYZE**: Read all tasks. Determine dependency graph (LLM reasoning). Call
   `task_set_dependency` for each edge.
3. **PLAN**: Group tasks into parallelizable waves. Decide agent count based on
   wave width. Create agent profiles with task assignments via
   `agent_profile_create`.
4. **SPAWN**: Call `agent_spawn` for each profile. Agents appear in cmux
   workspaces.
5. **WAIT**: Lead goes idle. The MCP server pushes messages into the lead's
   session via the OpenCode SDK when events occur (task completions, conflicts,
   crashes, etc.). Lead reacts only when prompted.
6. **CONVERGE**: All tasks complete, all branches merged. Call `export_tasks` to
   update tasks.md. Call `team_end` to clean up. Offer to archive the OpenSpec
   change.

**The lead does not poll.** The MCP server is the active coordinator:
- When a worker completes a task → server pushes status update to lead's session
- When a conflict occurs → server pushes conflict details to lead's session
- When an agent crashes → server pushes crash report to lead's session
- When all tasks are done → server pushes "ready to converge" to lead's session

**Mid-session re-plans**: The human interrupts the lead (same OpenCode session).
The lead calls `reimport_tasks`, reviews the diff, pauses affected agents via
`send_message`, adjusts assignments, and resumes.

### Q2: Worker Agent Instructions (Resolved)

Workers are autonomous OpenCode instances with a custom agent definition.

**Three-layer knowledge stack:**

1. **Agent Definition (GUARANTEED)**: `.opencode/agents/worker.md` written into
   the worktree by the MCP server before spawning. Contains the core behavioral
   loop, MCP tool usage patterns, conflict negotiation protocol, and startup
   sequence.

2. **MCP Dynamic Context (GUARANTEED)**: `agent_whoami` response provides agent
   name, team name, assigned task list with descriptions, OpenSpec change context
   (proposal summary), and design decisions relevant to the agent's tasks.

3. **Skills (ON DEMAND)**: `openspec-apply-change` can be loaded if the agent
   needs OpenSpec workflow guidance. Not critical — agent can work without it.

**Worker behavioral loop** (defined in agent definition):
```
1. Call agent_whoami → get name, tasks, context
2. Call task_claim on first available task
3. Implement the task (edit files, run tests)
4. Call task_complete when done
5. If messages arrive (pushed by MCP server via SDK):
   - Merge notifications → decide whether to rebase
   - Conflict negotiation → read diffs, adjust code, reply
6. Repeat until all tasks done
7. Call merge_to_main
8. If conflict → negotiate with counterpart agent
9. If success → notify #team
```

**Workers do NOT poll for messages.** The MCP server pushes messages directly
into the worker's OpenCode session via `POST /session/{id}/prompt`. The worker
receives them as new messages in its conversation and reacts.

**Git knowledge**: MCP tools abstract the coordination-level git operations
(`merge_to_main`, `rebase_from_main`, `get_conflict_details`, `get_agent_diff`).
Workers use the shell for routine git operations (commit, diff, status) within
their own worktree.

### Q3: Rust Crate Structure (Resolved)

**MCP crate**: `rmcp` v1.2 — the official Rust MCP SDK. Supports both stdio and
streamable HTTP transport from a single `ServerHandler` implementation. Uses
`#[tool]` proc macros for tool definitions.

**Async runtime**: `tokio`

**Git operations**: `git2` for worktree creation/removal and branch management.
Shell out to `git` CLI for merge and rebase (libgit2's merge/rebase APIs are
incomplete compared to the CLI). Agents use git CLI directly for routine
operations within their worktrees.

**SQLite**: `rusqlite` with `r2d2` connection pool. Single-writer
(`Mutex<Connection>` with `BEGIN IMMEDIATE`) + reader pool pattern.

**cmux integration**: Unix socket client using `tokio::net::UnixStream` with
`serde_json` for JSON-RPC v2 messages.

**OpenCode SDK integration**: HTTP client (e.g., `reqwest`) for pushing messages
to agent sessions and subscribing to SSE event streams.

**Project structure**: Single crate with modules.

```
spook-teams/
├── Cargo.toml
└── src/
    ├── main.rs           ← CLI args, dual transport setup
    ├── server.rs         ← MCP ServerHandler + tool definitions
    ├── db.rs             ← SQLite schema, migrations, queries
    ├── team.rs           ← team lifecycle logic
    ├── task.rs           ← task engine (CRUD, DAG, claiming)
    ├── message.rs        ← message bus logic
    ├── worktree.rs       ← git worktree operations (git2 + CLI)
    ├── opencode.rs       ← OpenCode SDK client (session mgmt, event stream)
    ├── event.rs          ← event dispatcher (reacts to state changes,
    │                        pushes to agents via SDK, updates cmux)
    ├── spawner/
    │   ├── mod.rs        ← Spawner trait
    │   ├── cmux.rs       ← cmux JSON-RPC socket implementation
    │   └── headless.rs   ← future fallback (no UI)
    ├── bridge.rs         ← OpenSpec tasks.md import/export
    └── config.rs         ← CLI args, env detection
```

### Q4: How Workers Get Their Behavioral Skill (Resolved)

**Hybrid: custom agent definition + MCP dynamic context + on-demand skills.**

The MCP server writes files into each worktree before spawning:

1. **`opencode.json`** — MCP server config pointing to the shared server:
   ```json
   {
     "mcp": {
       "spook-teams": {
         "type": "remote",
         "url": "http://localhost:<port>",
         "headers": { "X-Agent-Profile": "<profile_id>" }
       }
     }
   }
   ```

2. **`.opencode/agents/worker.md`** — custom agent definition with full
   behavioral prompt (static, same for all workers). Defines the worker's
   personality, work loop, conflict protocol, and MCP tool usage.

**Worker spawning uses OpenCode's serve + attach model:**

1. MCP server spawns `opencode serve --port <port> --dir <worktree>` as a
   managed child process (not in a cmux pane).
2. MCP server probes readiness (brief retry on health endpoint — the only
   acceptable "poll" in the system, during startup only).
3. MCP server creates session via SDK: `POST /session { agent: "worker" }`.
4. MCP server sends initial prompt via SDK:
   `POST /session/{id}/prompt { "Begin working on your assigned tasks." }`.
5. MCP server subscribes to `GET /event` SSE stream for crash/idle detection.
6. MCP server creates cmux workspace with split pane.
7. Left surface: `opencode attach <url> -s <session_id>` (interactive TUI).
8. Right surface: shell in worktree directory.
9. MCP server sets cmux sidebar status + progress.

The human sees a fully interactive TUI for each agent and can observe, interact
with, or intervene in any agent's session.

---

## Reactive Event Architecture

The system is fully reactive — no polling anywhere. Three push channels work
together:

### 1. MCP Tool Calls (Agent → Server)

Agents call MCP tools (`task_complete`, `merge_to_main`, `send_message`, etc.).
Each tool call triggers server-side logic that may cascade into pushes to other
agents.

### 2. OpenCode SDK Push (Server → Agent)

The MCP server uses the OpenCode HTTP API to inject messages directly into agent
sessions:
- `POST /session/{id}/prompt` — delivers a new message to the agent's
  conversation
- The agent sees it immediately and reacts

This is used for:
- Notifying the lead of task completions, crashes, conflicts
- Delivering messages between agents (peer-to-peer, routed through server)
- Broadcasting merge notifications to all agents
- Pushing new task assignments to unblocked agents

### 3. OpenCode SDK Event Stream (Agent → Server, passive)

The MCP server subscribes to each agent's `GET /event` SSE stream to detect:
- `session.idle` — agent finished processing, may be done with tasks
- `session.error` — agent encountered an error
- Process exit — managed child process terminated (crash detection)

### Event Flow Diagram

```
Worker calls task_complete (MCP tool)
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│  MCP Server: Event Dispatcher                            │
│                                                          │
│  1. Update task status in SQLite                         │
│  2. Unblock dependent tasks                              │
│  3. Update tasks.md projection                           │
│  4. Push to lead session (SDK):                          │
│     "Agent alice completed task 1.2 (3/7 done)"         │
│  5. Push to unblocked agents (SDK):                      │
│     "Task 2.1 is now available"                          │
│  6. Update cmux sidebar:                                 │
│     set-progress 0.42 --label "3/7 tasks"               │
│     log --level success "Completed task 1.2"            │
│  7. If all tasks done:                                   │
│     Push to lead: "All tasks complete. Ready to merge."  │
└─────────────────────────────────────────────────────────┘
```

```
Worker calls merge_to_main (MCP tool) → CONFLICT
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│  MCP Server: Event Dispatcher                            │
│                                                          │
│  1. Record conflict in SQLite                            │
│  2. Query file_changes: who else touched these files?    │
│  3. Push to conflicting agent (SDK):                     │
│     "Alice's merge conflicts with your changes to        │
│      auth.rs. Alice needs you to adjust. Here's her      │
│      diff: ..."                                          │
│  4. Push to lead (SDK):                                  │
│     "Conflict detected: alice vs bob on auth.rs.         │
│      Agents are negotiating."                            │
│  5. Update cmux sidebar:                                 │
│     set-status conflict "auth.rs" --icon warning         │
│     log --level warning "Merge conflict: alice vs bob"   │
│     notify --title "Conflict" --body "alice vs bob"      │
└─────────────────────────────────────────────────────────┘
```

```
Agent process exits unexpectedly
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│  MCP Server: Child Process Monitor                       │
│                                                          │
│  1. Detect child exit (tokio child handle returns)       │
│  2. Mark agent as crashed in SQLite                      │
│  3. Mark agent's in-progress tasks as failed             │
│  4. Capture last terminal output from cmux:              │
│     read-screen --workspace <id> --scrollback            │
│  5. Record crash event with context in agent_events      │
│  6. Push to lead (SDK):                                  │
│     "Agent bob crashed while working on task 3.2.        │
│      Last output: [terminal snapshot]. Branch            │
│      teams/bob preserved. Respawn?"                      │
│  7. Update cmux sidebar:                                 │
│     set-status agent "Crashed" --icon xmark --color red  │
└─────────────────────────────────────────────────────────┘
```

---

## OpenCode Agent Spawning Model

Workers run as `opencode serve` child processes managed by the MCP server. The
interactive TUI is provided by `opencode attach` in a cmux pane. This separates
the AI backend (programmatically controlled) from the human-visible UI.

### Per-Agent Process Architecture

```
MCP Server (parent process)
    │
    ├── Child: opencode serve --port 4097 --dir /project/worktrees/alice
    │   (headless, no UI, managed lifecycle)
    │   (MCP server subscribes to GET /event SSE stream)
    │   (MCP server pushes via POST /session/{id}/prompt)
    │
    ├── Child: opencode serve --port 4098 --dir /project/worktrees/bob
    │   (same pattern)
    │
    └── cmux workspaces (UI only):
        │
        ├── Workspace "agent-alice":
        │   ┌───────────────────────┬───────────────────────┐
        │   │ opencode attach       │ Shell                 │
        │   │ http://localhost:4097 │ /project/worktrees/   │
        │   │ -s <alice_session>    │ alice                 │
        │   │                       │                       │
        │   │ (interactive TUI,     │ (user can inspect     │
        │   │  user can observe     │  files, run tests,    │
        │   │  and interact)        │  check git status)    │
        │   └───────────────────────┴───────────────────────┘
        │
        └── Workspace "agent-bob":
            ┌───────────────────────┬───────────────────────┐
            │ opencode attach       │ Shell                 │
            │ http://localhost:4098 │ /project/worktrees/   │
            │ -s <bob_session>      │ bob                   │
            └───────────────────────┴───────────────────────┘
```

### Agent Spawn Sequence

```
1.  MCP server picks a free port (e.g., 4097)
2.  MCP server writes opencode.json + .opencode/agents/worker.md into worktree
3.  MCP server spawns: opencode serve --port 4097 --dir <worktree>
    (as managed child process, stderr captured for diagnostics)
4.  MCP server probes readiness (brief retry on health endpoint)
5.  MCP server creates session: POST /session { agent: "worker" }
6.  MCP server sends initial prompt: POST /session/{id}/prompt
    { "Begin working on your assigned tasks." }
7.  MCP server subscribes to GET /event SSE stream (crash/idle detection)
8.  MCP server records opencode_url + session_id in SQLite members table
9.  cmux new-workspace --cwd <worktree_path> → workspace_id
10. cmux rename-workspace --workspace <id> "agent-<name>"
11. cmux new-split right --workspace <id> → right surface
12. cmux send --surface <left_id>
    "opencode attach http://localhost:4097 -s <session_id>\n"
13. cmux set-status agent "Working" --icon bolt --workspace <id>
14. cmux set-progress 0.0 --label "0/N tasks" --workspace <id>
```

---

## SQLite Schema (Draft)

```sql
CREATE TABLE teams (
    id TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    openspec_change TEXT NOT NULL,
    status TEXT CHECK(status IN ('active','paused','completed')) NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE members (
    profile_id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id),
    name TEXT NOT NULL,
    role TEXT CHECK(role IN ('lead','worker')) NOT NULL,
    worktree_path TEXT,
    branch TEXT,
    opencode_url TEXT,                 -- http://localhost:<port>
    opencode_session_id TEXT,          -- session UUID
    cmux_workspace_id TEXT,            -- cmux workspace for sidebar updates
    status TEXT CHECK(status IN (
        'pending_spawn','active','idle','crashed','killed','completed'
    )) NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id),
    source_id TEXT NOT NULL,          -- e.g. "1.1", "2.3" from tasks.md
    title TEXT NOT NULL,
    description TEXT,
    status TEXT CHECK(status IN (
        'pending','blocked','in_progress','completed','failed','cancelled'
    )) NOT NULL,
    owner TEXT REFERENCES members(profile_id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    depends_on TEXT NOT NULL REFERENCES tasks(id),
    PRIMARY KEY (task_id, depends_on)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    sender TEXT NOT NULL,
    recipient TEXT,                    -- NULL = broadcast to topic
    topic TEXT,                        -- '#team', '#conflict', '@alice'
    message_type TEXT NOT NULL,        -- 'text', 'conflict_negotiation',
                                      -- 'merge_notification', 'system'
    body TEXT NOT NULL,
    read BOOLEAN DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE worktrees (
    profile_id TEXT PRIMARY KEY REFERENCES members(profile_id),
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    status TEXT CHECK(status IN (
        'active','merging','merged','conflict','cleaned_up'
    )) NOT NULL,
    base_commit TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE file_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    agent_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    task_id TEXT REFERENCES tasks(id),
    change_type TEXT CHECK(change_type IN ('added','modified','deleted')),
    timestamp TEXT NOT NULL
);
CREATE INDEX idx_file_changes_path ON file_changes(file_path);

CREATE TABLE agent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    team_id TEXT NOT NULL REFERENCES teams(id),
    agent_name TEXT NOT NULL,
    event_type TEXT NOT NULL,          -- 'spawned', 'crashed', 'completed',
                                      -- 'killed', 'merge_started',
                                      -- 'merge_completed', 'conflict_detected'
    task_id TEXT,
    details TEXT,                      -- JSON blob: terminal output, error, etc.
    timestamp TEXT NOT NULL
);
```

---

## MCP Tools (Draft)

### Team Lifecycle
| Tool | Description |
|------|-------------|
| `team_create` | Create team from OpenSpec change. Imports tasks, creates SQLite DB, starts HTTP listener. |
| `team_status` | Overview: agents, tasks, progress, active conflicts. |
| `team_end` | Export tasks.md, clean up worktrees, shut down. |

### Agent Profiles
| Tool | Description |
|------|-------------|
| `agent_profile_create` | Create profile: name, assigned tasks. Creates worktree, branch, writes opencode.json + agent def. Returns profile_id. |
| `agent_spawn` | Spawn opencode serve as child process, create session via SDK, create cmux workspace with attach + shell. Sets sidebar status. |
| `agent_whoami` | Worker calls this on startup. Returns name, tasks, worktree path, team context, instructions. |
| `agent_status` | Health, current task, progress, worktree state. |
| `agent_kill` | Kill child process, preserve branch, mark tasks failed, record crash context. |

### Tasks
| Tool | Description |
|------|-------------|
| `task_list` | List tasks. Filter: mine, all, available (unblocked + unowned). |
| `task_set_dependency` | Record dependency edge between tasks. |
| `task_claim` | Atomic claim of a pending/unblocked task. |
| `task_complete` | Mark done, unblock dependents, update tasks.md projection. Triggers event dispatcher: notify lead, notify unblocked agents, update cmux sidebar. |
| `task_fail` | Mark failed with reason. Triggers: notify lead, update cmux. |

### Messages
| Tool | Description |
|------|-------------|
| `send_message` | Send to `@agent-name` or `#topic`. Server routes via SDK push to recipient's session. |
| `read_inbox` | Read messages for calling agent. Optional unread_only filter. (Mostly for catch-up; primary delivery is via SDK push.) |

### Git / Worktree
| Tool | Description |
|------|-------------|
| `worktree_status` | Branch state, divergence from main, modified files. |
| `merge_to_main` | Attempt merge. On success: broadcast to #team via SDK push. On conflict: notify counterpart + lead via SDK push. |
| `get_conflict_details` | Conflicting files, both sides, counterpart agents. |
| `get_agent_diff` | Read another agent's changes (for conflict negotiation). |
| `rebase_from_main` | Rebase current branch onto main. |

### OpenSpec Bridge
| Tool | Description |
|------|-------------|
| `reimport_tasks` | Re-read tasks.md, diff against current state, return changes for review. |
| `export_tasks` | Write current SQLite state to tasks.md. |

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        spook-teams MCP Server                       │
│                              (Rust)                                 │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐    │
│  │ Team Mgmt   │  │ Task Engine │  │ Message Bus             │    │
│  │             │  │             │  │                         │    │
│  │ create/     │  │ CRUD +      │  │ peer-to-peer DMs       │    │
│  │ destroy     │  │ dependency  │  │ topic broadcasts       │    │
│  │ profiles    │  │ DAG + claim │  │ conflict negotiation   │    │
│  │ spawn/kill  │  │ import from │  │ (all routed via SDK    │    │
│  │             │  │ tasks.md    │  │  push to sessions)     │    │
│  └──────┬──────┘  └──────┬──────┘  └───────────┬─────────────┘    │
│         │                │                      │                   │
│         └────────────────┼──────────────────────┘                   │
│                          │                                          │
│  ┌───────────────────────┼──────────────────────────────────────┐  │
│  │            Event Dispatcher (reactive core)                  │  │
│  │                                                              │  │
│  │  Triggered by: MCP tool calls, SDK event stream, child exit  │  │
│  │  Actions: SDK push to sessions, cmux sidebar updates,        │  │
│  │           SQLite state transitions, task unblocking           │  │
│  └───────────────────────┼──────────────────────────────────────┘  │
│                          │                                          │
│                   ┌──────┴──────┐                                   │
│                   │   SQLite    │     ┌─────────────────────────┐  │
│                   │  (WAL mode) │     │  Spawner Trait          │  │
│                   │             │     │  ┌───────┐ ┌─────────┐ │  │
│                   │ teams       │     │  │ cmux  │ │headless │ │  │
│                   │ members     │     │  │(impl) │ │(future) │ │  │
│                   │ tasks       │     │  └───────┘ └─────────┘ │  │
│                   │ messages    │     └─────────────────────────┘  │
│                   │ worktrees   │                                   │
│                   │ file_changes│     ┌─────────────────────────┐  │
│                   │ agent_events│     │  OpenCode SDK Client    │  │
│                   └─────────────┘     │                         │  │
│                                       │  Per-agent:             │  │
│  ┌────────────────────────────────┐   │  - SSE event stream     │  │
│  │  Worktree Manager              │   │  - Session push API     │  │
│  │  (git2 + git CLI)              │   │  - Health monitoring    │  │
│  │  create / remove / merge /     │   └─────────────────────────┘  │
│  │  rebase / conflict-detect      │                                 │
│  └────────────────────────────────┘   ┌─────────────────────────┐  │
│                                       │  Child Process Manager  │  │
│  ┌────────────────────────────────┐   │                         │  │
│  │  OpenSpec Bridge               │   │  opencode serve procs   │  │
│  │  import tasks.md → SQLite      │   │  lifecycle management   │  │
│  │  export SQLite → tasks.md      │   │  crash detection        │  │
│  └────────────────────────────────┘   └─────────────────────────┘  │
│                                                                     │
├───────────────────────┬─────────────────────────────────────────────┤
│   stdio (lead agent)  │      HTTP :PORT (worker MCP connections)    │
└───────────┬───────────┴──────────┬──────────────────┬───────────────┘
            │                      │                  │
     ┌──────┴──────┐       ┌──────┴──────┐   ┌──────┴──────┐
     │ Lead Agent  │       │ Agent Alice │   │ Agent Bob   │
     │ (user's     │       │ (serve:4097)│   │ (serve:4098)│
     │  OpenCode)  │       │ (cmux ws)   │   │ (cmux ws)   │
     └─────────────┘       └─────────────┘   └─────────────┘
```

---

## Rust Dependencies

```toml
[dependencies]
# MCP protocol (official Rust SDK)
rmcp = { version = "1.2", features = [
    "server",
    "transport-io",
    "transport-streamable-http-server",
] }

# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["rt"] }

# HTTP server (required by rmcp streamable HTTP transport)
axum = "0.8"

# HTTP client (for OpenCode SDK API calls + SSE event streams)
reqwest = { version = "0.12", features = ["json", "stream"] }

# Database
rusqlite = { version = "0.34", features = ["bundled"] }
r2d2 = "0.8"
r2d2_sqlite = "0.25"

# Git operations
git2 = "0.20"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "0.8"          # required by rmcp for tool parameter schemas

# CLI
clap = { version = "4", features = ["derive"] }

# Utilities
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
```

---

## cmux Integration Notes

### Spawning an Agent (revised for serve + attach model)
```
1.  MCP server picks a free port (e.g., 4097)
2.  MCP server writes opencode.json + .opencode/agents/worker.md into worktree
3.  MCP server spawns: opencode serve --port 4097 --dir <worktree>
    (as managed child process, stderr captured for diagnostics)
4.  MCP server probes readiness (brief retry on health endpoint)
5.  MCP server creates session: POST /session { agent: "worker" }
6.  MCP server sends initial prompt: POST /session/{id}/prompt
7.  MCP server subscribes to GET /event SSE stream
8.  MCP server records opencode_url + session_id in SQLite
9.  cmux new-workspace --cwd <worktree_path> → workspace_id
10. cmux rename-workspace --workspace <id> "agent-<name>"
11. cmux new-split right --workspace <id>
12. cmux send --surface <left_id>
    "opencode attach http://localhost:4097 -s <session_id>\n"
13. cmux set-status agent "Working" --icon bolt --workspace <id>
14. cmux set-progress 0.0 --label "0/N tasks" --workspace <id>
```

### Updating Status During Execution (triggered by event dispatcher)
```
cmux set-status task "Working on 1.2" --icon hammer --workspace <id>
cmux set-progress 0.42 --label "3/7 tasks" --workspace <id>
cmux log "Claimed task 1.2: JWT token generation" --workspace <id>
cmux log --level success "Completed task 1.1" --workspace <id>
cmux log --level warning "Merge conflict detected in auth.rs" --workspace <id>
cmux notify --title "agent-alice" --body "Conflict — needs negotiation"
```
