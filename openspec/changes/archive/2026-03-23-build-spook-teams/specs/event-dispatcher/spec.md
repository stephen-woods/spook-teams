## ADDED Requirements

### Requirement: Dispatch events on task completion
The system SHALL trigger cascading side effects when a task is completed: update SQLite, unblock dependent tasks, push status to the lead, push availability to unblocked agents, and update cmux sidebar.

#### Scenario: Task completion with dependents
- **WHEN** `task_complete` is called for task `1.1` and tasks `2.1` and `2.2` depend only on `1.1`
- **THEN** the event dispatcher updates task `1.1` to `completed`, changes tasks `2.1` and `2.2` from `blocked` to `pending`, pushes a status message to the lead's session (`"Agent alice completed task 1.1 (3/7 done)"`), pushes availability messages to agents assigned `2.1` and `2.2`, and updates cmux sidebar progress

#### Scenario: All tasks complete
- **WHEN** the last task is marked complete
- **THEN** the event dispatcher pushes a convergence message to the lead: `"All tasks complete. Ready to merge and converge."`

### Requirement: Dispatch events on merge conflict
The system SHALL trigger conflict notification when a merge attempt fails: record the conflict, notify the counterpart agent(s) with diff context, notify the lead, and update cmux sidebar.

#### Scenario: Conflict between two agents
- **WHEN** agent `alice` calls `merge_to_main` and conflicts with `bob`'s merged changes on `auth.rs`
- **THEN** the event dispatcher pushes a conflict_negotiation message to `bob`'s session with alice's diff and the conflicting files, pushes a status message to the lead (`"Conflict detected: alice vs bob on auth.rs. Agents are negotiating."`), and updates cmux sidebar with a warning status

### Requirement: Dispatch events on agent crash
The system SHALL trigger crash handling when an agent's process exits unexpectedly: mark the agent as crashed, mark tasks as failed, capture context, and notify the lead.

#### Scenario: Agent process exits unexpectedly
- **WHEN** the child process monitor detects that agent `bob`'s `opencode serve` process has exited with non-zero status
- **THEN** the event dispatcher marks `bob` as `crashed`, marks `bob`'s in-progress tasks as `failed`, captures terminal output from cmux via `read-screen`, records a crash event in `agent_events`, pushes a crash report to the lead's session with context (last task, terminal output, branch name), and updates cmux sidebar to `Crashed`

### Requirement: Dispatch events on task failure
The system SHALL trigger lead notification when a task is explicitly failed by an agent.

#### Scenario: Agent reports task failure
- **WHEN** agent `alice` calls `task_fail` for task `1.2` with reason `"tests failing after auth refactor"`
- **THEN** the event dispatcher pushes a failure notification to the lead's session with the task details and reason, and updates cmux sidebar log

### Requirement: Dispatch events on merge success
The system SHALL broadcast merge notifications to all agents when a branch is successfully merged to main, so agents can decide whether to rebase.

#### Scenario: Successful merge broadcast
- **WHEN** agent `alice` successfully merges to main, changing files `src/auth.rs` and `src/config.rs`
- **THEN** the event dispatcher pushes a merge notification to all other active agents' sessions listing the changed files and commit range, and updates cmux sidebar with a success log entry
