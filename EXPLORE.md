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

## Open Questions

### Q1: The `apply-teams` Skill — What Does the Lead's Orchestration Look Like?

The lead agent runs an `apply-teams` skill that orchestrates the entire team
session. This skill needs to be designed. Key sub-questions:

- What is the full lifecycle the skill manages? (import → analyze → plan waves →
  spawn → monitor → merge → export → archive)
- How does the lead monitor progress? Polling `team_status`? Waiting for
  messages?
- How does the lead handle the "all tasks complete" state?
- How does the lead handle mid-session re-plans when specs change?
- What does the lead's behavioral prompt look like?

### Q2: Worker Agent Instructions — What Does a Worker Know and Do?

Each worker agent is an independent OpenCode instance. It needs to know how to
behave: claim tasks, do work, use MCP tools for coordination, handle conflicts.
Key sub-questions:

- Does the worker use a modified OpenSpec `apply` skill, or a completely new
  `apply-worker` skill?
- How does the worker know to check for messages / merge notifications?
- What is the worker's behavioral loop? (claim → work → complete → check for
  next → merge when done)
- How much git knowledge does the worker need? Should MCP tools abstract git
  operations, or does the worker use git directly?
- How does the worker handle the conflict negotiation protocol? What does its
  prompt look like when a conflict is detected?

### Q3: Rust Crate Structure — What Does the Implementation Look Like?

The MCP server is a Rust binary. Key sub-questions:

- Which MCP crate to use? (`rmcp`, `mcp-server`, or raw implementation over
  HTTP?)
- Async runtime: `tokio`?
- Git operations: `git2-rs` (libgit2 bindings) or shell out to `git` CLI?
- SQLite: `rusqlite` with `r2d2` pool? Single-writer + reader pool pattern?
- cmux integration: Unix socket client with `serde_json` for JSON-RPC?
- Project structure: workspace with multiple crates? Single crate with modules?

### Q4: How Workers Get Their Behavioral Skill

Workers need instructions on how to behave as team members. Options include:

- **MCP-delivered instructions**: `agent_whoami` returns a full behavioral prompt
  that the worker's OpenCode session uses as its initial context.
- **Skill file in worktree**: The server writes a `.opencode/skills/apply-worker/SKILL.md`
  into the worktree before spawning.
- **Hybrid**: Skill file provides the static behavioral framework, MCP tools
  provide dynamic context (current tasks, team state, instructions).
- How does the worker's initial prompt get set? Does the cmux spawner send an
  initial message to OpenCode, or does the skill auto-activate?

---

## Rust Dependencies (Preliminary)

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.34", features = ["bundled"] }
r2d2 = "0.8"
r2d2_sqlite = "0.25"
git2 = "0.20"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
# MCP crate TBD — needs research
# HTTP server for worker transport — axum or similar
```

---

## cmux Integration Notes

### Spawning an Agent
```
1. cmux new-workspace --cwd <worktree_path>    → workspace_id
2. cmux rename-workspace --workspace <id> "agent-<name>"
3. cmux new-split right --workspace <id>        → creates right pane
4. cmux send --surface <left_surface> "opencode\n"
5. cmux set-status agent "Initializing" --icon bolt --workspace <id>
6. cmux set-progress 0.0 --label "0/N tasks" --workspace <id>
```

### Updating Status During Execution
```
cmux set-status task "Working on 1.2" --icon hammer --workspace <id>
cmux set-progress 0.42 --label "3/7 tasks" --workspace <id>
cmux log "Claimed task 1.2: JWT token generation" --workspace <id>
cmux log --level success "Completed task 1.1" --workspace <id>
cmux log --level warning "Merge conflict detected in auth.rs" --workspace <id>
cmux notify --title "agent-alice" --body "Conflict — needs negotiation"
```
