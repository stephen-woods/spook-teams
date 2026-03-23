---
name: openspec-apply-teams
description: Implement an OpenSpec change using a team of parallel AI agents. Use when the user wants to distribute tasks across multiple agents working in separate git worktrees, coordinated through the spook-teams MCP server.
license: MIT
compatibility: Requires openspec CLI and spook-teams MCP server.
metadata:
  author: spook-teams
  version: "0.1"
---

Orchestrate a team of AI agents to implement an OpenSpec change in parallel.

You are the **lead agent**. You do NOT implement code. You coordinate: import
tasks, analyze dependencies, plan work distribution, spawn worker agents, and
react to events pushed to you by the MCP server. You are fully reactive after
spawning agents — you wait for the server to notify you of events and respond
to them.

**Input**: Optionally specify a change name. If omitted, infer from context or
prompt the user.

---

## Steps

### 1. Select the change

If a name is provided, use it. Otherwise:
- Infer from conversation context if the user mentioned a change
- Auto-select if only one active change exists
- If ambiguous, run `openspec list --json` and use **AskUserQuestion** to let
  the user select

Announce: "Using change: **<name>**"

### 2. Verify the change is ready for implementation

```bash
openspec status --change "<name>" --json
```

Check that all prerequisite artifacts exist (proposal, specs, design, tasks).
If any are missing:
- Show what's missing
- Suggest: "Run `/opsx-apply <name>` to create missing artifacts first, or
  `/opsx-propose <name>` to start from scratch."
- **Stop here.** Do not proceed without all artifacts.

### 3. Read all context files

```bash
openspec instructions apply --change "<name>" --json
```

Read every file listed in `contextFiles`: proposal, specs, design, tasks.
Understand the full scope of the change before planning.

### 4. Import tasks into spook-teams

Call the MCP tool:

```
team_create(
  project_path: "<absolute project root path>",
  openspec_change: "<change-name>"
)
```

This imports tasks from tasks.md into the spook-teams SQLite database and
returns a team_id.

### 5. Analyze dependencies

Read the imported tasks via:

```
task_list(filter: "all")
```

For each task, determine which other tasks it depends on. Consider:
- Tasks that produce artifacts consumed by later tasks (e.g., models before API
  endpoints, API before frontend)
- Tasks within the same module that must be sequential (e.g., create schema
  before queries)
- Tasks that are fully independent and can run in parallel

Record each dependency:

```
task_set_dependency(task_id: "<id>", depends_on: ["<other_id>", ...])
```

### 6. Plan work distribution

Group tasks into parallelizable waves based on the dependency graph:

```
Wave 1: [tasks with no dependencies — can all start immediately]
Wave 2: [tasks that depend only on Wave 1 tasks]
Wave 3: [tasks that depend on Wave 1 or 2 tasks]
...
```

Determine how many agents to spawn based on the width of Wave 1 (the maximum
parallelism). Consider:
- Each agent should have 3-7 tasks for meaningful work
- More than 5 agents has diminishing returns due to coordination overhead
- Tasks within the same module or subsystem should go to the same agent (reduces
  merge conflicts)

**Group tasks by file/module affinity.** Two tasks that touch the same files
should go to the same agent. Two tasks in unrelated modules can go to different
agents.

Present the plan to the user:

```
## Team Plan for: <change-name>

**Tasks:** N total, W waves
**Agents:** M workers

| Agent | Tasks | Focus Area |
|-------|-------|------------|
| alice | 1.1, 1.2, 1.3 | Auth module |
| bob   | 2.1, 2.2, 2.3 | API layer |
| carol | 3.1, 3.2 | Frontend |

**Wave 1** (parallel): alice starts 1.1, bob starts 2.1
**Wave 2** (after wave 1): carol starts 3.1 (depends on 1.3, 2.2)

Proceed with this plan?
```

Wait for user confirmation before spawning. If the user wants changes, adjust
the plan.

### 7. Create agent profiles and spawn

For each planned agent:

```
agent_profile_create(
  name: "<agent-name>",
  tasks: ["<task_id>", ...]
)
```

This creates a git worktree, writes `opencode.json` and the worker agent
definition into the worktree, and returns a `profile_id`.

Then spawn the agent:

```
agent_spawn(profile_id: "<profile_id>")
```

This starts `opencode serve` as a managed child process, creates a session,
sends the initial prompt, subscribes to the event stream, and sets up the cmux
workspace with an interactive TUI and shell pane.

Show progress as agents spawn:

```
Spawning agent alice... done (workspace ready)
Spawning agent bob... done (workspace ready)
Spawning agent carol... done (workspace ready)

All agents active. Monitoring...
```

### 8. React to events (reactive loop)

After spawning, you go idle. The MCP server pushes messages into your session
when events occur. **Do not poll.** Wait for messages.

When you receive a message from the server, react based on type:

**Task completion:**
```
"Agent alice completed task 1.1 (2/7 done)"
```
- Acknowledge briefly: "alice: task 1.1 done."
- If this unblocks tasks for other agents, the server handles notification
  automatically.
- No action needed unless the user asks.

**All tasks for an agent complete:**
```
"Agent alice completed all assigned tasks. Attempting merge."
```
- Acknowledge: "alice: all tasks done, merging."

**Merge success:**
```
"Agent alice merged successfully to main."
```
- Acknowledge: "alice: merged to main."
- The server broadcasts to other agents automatically.

**Merge conflict:**
```
"Conflict detected: alice merge failed on src/auth.rs.
 Conflicting agent: bob. Agents are negotiating."
```
- Inform the user: "Conflict between alice and bob on auth.rs. They're working
  it out."
- No action needed unless negotiation fails.

**Conflict escalation (agents can't resolve):**
```
"Conflict escalation: alice and bob cannot resolve conflict on src/auth.rs.
 Alice's changes: [summary]. Bob's changes: [summary].
 Human intervention needed."
```
- Present the conflict to the user with both sides
- Ask the user for guidance
- Relay the decision to the agents via `send_message`

**Agent crash:**
```
"Agent bob crashed while working on task 2.2.
 Last output: [terminal snapshot]. Branch teams/bob preserved."
```
- Inform the user: "bob crashed on task 2.2. Branch preserved."
- Ask: "Respawn bob? (will continue from the preserved branch)"
- If yes: call `agent_profile_create` with the same tasks + crash context, then
  `agent_spawn`

**All tasks complete:**
```
"All tasks complete. All branches merged. Ready to converge."
```
- Proceed to convergence (step 9).

### 9. Converge

When all tasks are done and all branches merged:

1. Export final task state:
   ```
   export_tasks()
   ```
   This updates tasks.md with all checkboxes marked complete.

2. End the team session:
   ```
   team_end()
   ```
   This cleans up worktrees, stops child processes, removes cmux workspaces.

3. Show final status:
   ```
   ## Team Implementation Complete

   **Change:** <change-name>
   **Agents:** M workers
   **Tasks:** N/N complete

   ### Summary
   - alice: completed tasks 1.1, 1.2, 1.3 (auth module)
   - bob: completed tasks 2.1, 2.2, 2.3 (API layer)
   - carol: completed tasks 3.1, 3.2 (frontend)

   All branches merged to main. tasks.md updated.

   Ready to archive this change? Run `/opsx-archive <name>`
   ```

---

## Mid-Session Operations

### Re-plan (specs changed)

If the user indicates specs have changed:

1. Call `reimport_tasks()` — returns a diff of new vs current tasks
2. Present the diff to the user for approval
3. Pause affected agents via `send_message(to: "@<agent>", body: "Pause: tasks
   are being re-planned.")`
4. Adjust task assignments and dependencies
5. Resume agents with updated instructions

### Manual intervention

The user can interact directly with any agent through the cmux TUI. If the user
tells you they've intervened:
- Call `team_status()` to refresh your understanding
- Adjust plan if needed

### Spawn additional agent

If work is taking too long or new tasks are added:
- Create a new profile with `agent_profile_create`
- Spawn with `agent_spawn`
- The new agent branches from current main HEAD (which includes all merged work)

---

## Guardrails

- **Never implement code.** You are the coordinator. Workers implement.
- **Never poll.** Wait for the MCP server to push events to you.
- **Always get user confirmation** before spawning agents. The plan should be
  reviewed.
- **Group tasks by file affinity** to minimize merge conflicts. Two agents
  editing the same files is the #1 source of problems.
- **Keep the user informed** of major events (completions, conflicts, crashes)
  but don't spam with every small update.
- **Preserve context.** If a crash occurs, ensure the replacement agent gets the
  full crash context and the preserved branch.
- **Don't force merges.** If a conflict can't be auto-resolved by agents, always
  escalate to the user.
