## ADDED Requirements

### Requirement: Spawn OpenCode serve instance
The system SHALL spawn `opencode serve` as a managed child process for each worker agent, with a unique port and the agent's worktree as the working directory.

#### Scenario: Successful spawn
- **WHEN** the system spawns an OpenCode instance for agent `alice` with port `4097` and worktree `/project/.worktrees/alice/`
- **THEN** the system executes `opencode serve --port 4097 --dir /project/.worktrees/alice/` as a child process, captures stderr for diagnostics, and monitors the process handle for unexpected exit

### Requirement: Probe health until ready
The system SHALL probe the OpenCode serve instance's health endpoint with brief retries until the server is ready to accept requests. This is the only acceptable polling behavior in the system.

#### Scenario: Server becomes ready
- **WHEN** the system probes the health endpoint and receives a successful response within the timeout
- **THEN** the system proceeds with session creation

#### Scenario: Server fails to start
- **WHEN** the health probe exceeds the retry timeout (e.g., 30 seconds)
- **THEN** the system kills the child process, marks the agent as `crashed`, records the startup failure, and notifies the lead

### Requirement: Create session with agent type
The system SHALL create an OpenCode session with the `worker` agent type after the serve instance is healthy.

#### Scenario: Session creation
- **WHEN** the health probe succeeds for agent `alice` on port `4097`
- **THEN** the system calls `POST /session { agent: "worker" }` and stores the returned session ID in the members table

### Requirement: Push prompt to agent session
The system SHALL push messages into an agent's OpenCode session via `POST /session/{id}/prompt`. This is the primary mechanism for delivering messages, notifications, and instructions to agents.

#### Scenario: Initial prompt
- **WHEN** a session is created for agent `alice`
- **THEN** the system pushes the initial prompt: `"Begin working on your assigned tasks."`

#### Scenario: Event notification
- **WHEN** the event dispatcher needs to notify agent `bob` of a merge
- **THEN** the system pushes the notification text to bob's session via `POST /session/{id}/prompt`

### Requirement: Subscribe to SSE event stream
The system SHALL subscribe to each agent's `GET /event` SSE stream to detect session events: idle, error, and completion.

#### Scenario: Detect session idle
- **WHEN** the SSE stream emits a `session.idle` event for agent `alice`
- **THEN** the system records the idle state and can use it for agent health monitoring

#### Scenario: SSE connection lost
- **WHEN** the SSE connection to an agent's event stream drops
- **THEN** the system attempts to reconnect with exponential backoff, and if the reconnection fails after retries, treats it as a potential crash

### Requirement: Monitor child process lifecycle
The system SHALL monitor the `opencode serve` child process for each agent and detect unexpected exits.

#### Scenario: Process exits unexpectedly
- **WHEN** the child process for agent `bob` exits with a non-zero status code
- **THEN** the system triggers the crash event dispatcher flow: mark crashed, mark tasks failed, capture context, notify lead

#### Scenario: Graceful shutdown
- **WHEN** `agent_kill` is called for agent `alice`
- **THEN** the system sends SIGTERM to the child process, waits briefly for exit, then SIGKILL if necessary, and cleans up resources

### Requirement: Manage port allocation
The system SHALL manage port allocation for `opencode serve` instances, ensuring each agent gets a unique port.

#### Scenario: Allocate ports
- **WHEN** the system needs to spawn 3 agents
- **THEN** the system allocates 3 unique ports starting from a configurable base port (default: 4097), checking each port for availability before assignment

#### Scenario: Port reuse after kill
- **WHEN** agent `alice` is killed and her port `4097` is released
- **THEN** the port becomes available for reuse by a replacement agent or new agent
