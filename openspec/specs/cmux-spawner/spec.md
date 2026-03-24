## ADDED Requirements

### Requirement: Create cmux workspace for agent
The system SHALL create a cmux workspace with a split pane layout when spawning an agent: OpenCode attach TUI on the left, shell on the right, both rooted in the agent's worktree directory.

#### Scenario: Workspace creation
- **WHEN** `agent_spawn` creates a workspace for agent `alice` with worktree at `/project/.worktrees/alice/`
- **THEN** the cmux spawner creates a new workspace named `agent-alice` with cwd set to the worktree, creates a right split for the shell pane, sends `opencode attach <url> -s <session_id>` to the left pane, sets initial sidebar status to `Working` with a bolt icon, and sets progress to `0.0` with label `0/N tasks`

### Requirement: Update sidebar status
The system SHALL update the cmux sidebar status for an agent's workspace to reflect current activity: working, idle, merging, conflict, crashed, completed.

#### Scenario: Status update on task claim
- **WHEN** the event dispatcher processes a task claim for agent `alice` on task `1.2`
- **THEN** the cmux spawner sets the workspace status to `Working on 1.2` with a hammer icon

#### Scenario: Status update on crash
- **WHEN** the event dispatcher processes a crash for agent `bob`
- **THEN** the cmux spawner sets the workspace status to `Crashed` with an xmark icon and red color

### Requirement: Update sidebar progress
The system SHALL update the cmux sidebar progress bar to reflect task completion ratio for each agent.

#### Scenario: Progress update
- **WHEN** agent `alice` completes her 3rd of 7 assigned tasks
- **THEN** the cmux spawner sets progress to `0.42` with label `3/7 tasks` on alice's workspace

### Requirement: Log events to sidebar
The system SHALL log significant events to the cmux sidebar log for each agent workspace.

#### Scenario: Log task completion
- **WHEN** agent `alice` completes a task
- **THEN** the cmux spawner logs a success-level entry: `"Completed task 1.1: <title>"`

#### Scenario: Log conflict
- **WHEN** a merge conflict is detected for agent `alice`
- **THEN** the cmux spawner logs a warning-level entry: `"Merge conflict: alice vs bob on auth.rs"`

### Requirement: Send desktop notifications
The system SHALL send cmux desktop notifications for critical events: conflicts, crashes, and team completion.

#### Scenario: Conflict notification
- **WHEN** a merge conflict is detected between agents
- **THEN** the cmux spawner sends a notification with title `"Conflict"` and body describing the agents and files involved

### Requirement: Read agent screen for crash context
The system SHALL read an agent's cmux terminal screen content (including scrollback) to capture context when an agent crashes.

#### Scenario: Capture crash output
- **WHEN** agent `bob` crashes and the system needs crash context
- **THEN** the cmux spawner reads the screen content from bob's workspace left pane (the OpenCode TUI pane) and returns the text for storage in agent_events

### Requirement: Destroy workspace on cleanup
The system SHALL destroy an agent's cmux workspace when the agent is removed or the team ends.

#### Scenario: Workspace cleanup
- **WHEN** `team_end` is called
- **THEN** the cmux spawner destroys all agent workspaces

### Requirement: Detect cmux availability
The system SHALL detect whether cmux is available by checking for the `CMUX_SOCKET_PATH` environment variable or known default socket paths. If unavailable, the system SHALL fall back to headless mode.

#### Scenario: cmux available
- **WHEN** the system starts and `CMUX_SOCKET_PATH` is set to a valid socket
- **THEN** the system uses the `CmuxSpawner` implementation for all workspace operations

#### Scenario: cmux unavailable
- **WHEN** the system starts and no cmux socket is found
- **THEN** the system logs a warning and operates in headless mode where all spawner operations are no-ops (agents still run, just without visual workspaces)
