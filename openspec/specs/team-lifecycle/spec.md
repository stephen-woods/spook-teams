## ADDED Requirements

### Requirement: Create team from OpenSpec change
The system SHALL create a new team session when the lead agent calls `team_create` with a project path and OpenSpec change name. The system SHALL import tasks from the change's `tasks.md` into SQLite, create the team record, register the lead agent as a member with role `lead`, and start the HTTP listener for worker connections.

#### Scenario: Successful team creation
- **WHEN** the lead agent calls `team_create` with a valid project path and existing OpenSpec change name
- **THEN** the system creates a team record with status `active`, imports all tasks from `tasks.md` into the tasks table with status `pending`, registers the lead as a member with role `lead`, and returns the team ID and imported task count

#### Scenario: Change has no tasks.md
- **WHEN** the lead agent calls `team_create` with a change that has no `tasks.md` file
- **THEN** the system returns an error indicating tasks.md was not found

#### Scenario: Team already exists for change
- **WHEN** a team already exists for the given change name with status `active`
- **THEN** the system returns an error indicating an active team already exists for this change

### Requirement: Query team status
The system SHALL provide a `team_status` tool that returns a comprehensive overview of the team including agent states, task progress, and active conflicts.

#### Scenario: Team status with active agents
- **WHEN** any agent calls `team_status`
- **THEN** the system returns: team status, list of all members with their current status and task, task counts by status (pending, blocked, in_progress, completed, failed), list of active conflicts, and overall progress percentage

### Requirement: End team session
The system SHALL provide a `team_end` tool that exports the final task state to `tasks.md`, optionally cleans up worktrees, and marks the team as completed.

#### Scenario: Clean shutdown with all tasks complete
- **WHEN** the lead agent calls `team_end` and all tasks are completed or cancelled
- **THEN** the system exports task states to `tasks.md`, removes all agent worktrees, marks the team as `completed`, and shuts down the HTTP listener

#### Scenario: End with incomplete tasks
- **WHEN** the lead agent calls `team_end` while tasks are still in progress
- **THEN** the system exports current task states to `tasks.md` (preserving in-progress states), kills active agents, cleans up worktrees, and marks the team as `completed`
