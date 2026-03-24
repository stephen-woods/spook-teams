## ADDED Requirements

### Requirement: Create agent profile
The system SHALL create an agent profile when the lead calls `agent_profile_create` with a name and list of assigned task IDs. The system SHALL create a git worktree, branch, write `opencode.json` and `.opencode/agents/worker.md` into the worktree, and return the profile ID.

#### Scenario: Successful profile creation
- **WHEN** the lead calls `agent_profile_create` with name `alice` and task IDs `["1.1", "1.2"]`
- **THEN** the system creates a worktree at `<project>/.worktrees/alice/`, creates branch `teams/alice` from current HEAD, writes `opencode.json` with remote MCP config pointing to the server's HTTP port and the agent's profile ID in the `X-Agent-Profile` header, writes `.opencode/agents/worker.md` from the worker agent template, registers the member with status `pending_spawn`, and returns the profile ID

#### Scenario: Duplicate agent name
- **WHEN** the lead calls `agent_profile_create` with a name that already exists in the team
- **THEN** the system returns an error indicating the agent name is already taken

### Requirement: Spawn agent
The system SHALL spawn an OpenCode serve instance and create a cmux workspace when the lead calls `agent_spawn` with a profile ID.

#### Scenario: Successful spawn
- **WHEN** the lead calls `agent_spawn` with a valid profile ID
- **THEN** the system spawns `opencode serve` on a free port in the agent's worktree directory, probes the health endpoint until ready, creates a session with agent type `worker`, sends the initial prompt, subscribes to the SSE event stream, creates a cmux workspace with split panes (attach TUI left, shell right), sets sidebar status to `Working`, sets progress to 0%, and updates the member status to `active`

#### Scenario: Spawn when cmux unavailable
- **WHEN** the lead calls `agent_spawn` but no cmux socket is detected
- **THEN** the system spawns the `opencode serve` instance headlessly (no cmux workspace), logs a warning, and the agent operates without a visual TUI

### Requirement: Agent self-identification
The system SHALL provide an `agent_whoami` tool that returns the calling agent's profile, assigned tasks, and team context.

#### Scenario: Worker identifies itself
- **WHEN** a worker agent calls `agent_whoami`
- **THEN** the system returns the agent's name, profile ID, assigned task list with titles and descriptions, worktree path, team name, OpenSpec change name, and a summary of the proposal context

### Requirement: Query agent status
The system SHALL provide an `agent_status` tool that returns detailed health and progress information for a specific agent.

#### Scenario: Query active agent
- **WHEN** any agent calls `agent_status` with a profile ID
- **THEN** the system returns the agent's name, status, current task (if any), completed task count, worktree branch state, and time since last activity

### Requirement: Kill agent
The system SHALL provide an `agent_kill` tool that terminates an agent's OpenCode process, preserves its branch, and records crash context.

#### Scenario: Kill active agent
- **WHEN** the lead calls `agent_kill` with a profile ID of an active agent
- **THEN** the system terminates the `opencode serve` child process, marks the agent's in-progress tasks as `failed` with reason `agent_killed`, preserves the worktree and branch, records the kill event with context (last task, modified files) in agent_events, updates cmux sidebar status to `Killed`, and updates the member status to `killed`

### Requirement: Crash detection and recovery
The system SHALL detect when an agent's `opencode serve` process exits unexpectedly and record crash context for recovery.

#### Scenario: Agent process crashes
- **WHEN** an agent's `opencode serve` child process exits with a non-zero status
- **THEN** the system marks the agent as `crashed`, marks its in-progress tasks as `failed` with reason `agent_crashed`, captures terminal output from cmux (if available), records the crash event with full context in agent_events, pushes a crash report to the lead agent's session, and updates cmux sidebar status to `Crashed`

#### Scenario: Respawn after crash
- **WHEN** the lead calls `agent_spawn` with a profile ID of a crashed agent
- **THEN** the system spawns a new `opencode serve` instance in the same worktree (preserving the agent's existing branch and partial work), creates a new session with crash context included in the initial prompt, and updates the member status to `active`
