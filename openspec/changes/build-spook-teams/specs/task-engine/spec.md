## ADDED Requirements

### Requirement: List tasks with filtering
The system SHALL provide a `task_list` tool that returns tasks filtered by scope: `mine` (caller's assigned tasks), `all` (all team tasks), or `available` (unblocked and unowned tasks).

#### Scenario: List available tasks
- **WHEN** a worker calls `task_list` with filter `available`
- **THEN** the system returns all tasks with status `pending` that have no unsatisfied dependencies and no current owner

#### Scenario: List my tasks
- **WHEN** a worker calls `task_list` with filter `mine`
- **THEN** the system returns all tasks assigned to or owned by the calling agent, regardless of status

### Requirement: Set task dependencies
The system SHALL provide a `task_set_dependency` tool that records a dependency edge between two tasks. A task with unsatisfied dependencies SHALL have status `blocked`.

#### Scenario: Create dependency
- **WHEN** the lead calls `task_set_dependency` with task_id `2.1` depends_on `1.1`
- **THEN** the system records the dependency edge, and if task `1.1` is not completed, task `2.1` is set to status `blocked`

#### Scenario: Circular dependency detection
- **WHEN** the lead calls `task_set_dependency` and the new edge would create a cycle in the dependency graph
- **THEN** the system returns an error indicating a circular dependency was detected and does not create the edge

### Requirement: Atomic task claiming
The system SHALL provide a `task_claim` tool that atomically assigns a task to the calling agent. Only tasks with status `pending` (no unsatisfied dependencies, no current owner) can be claimed.

#### Scenario: Successful claim
- **WHEN** a worker calls `task_claim` for a task with status `pending` and no unsatisfied dependencies
- **THEN** the system atomically sets the task's owner to the calling agent and status to `in_progress`, and returns the task details

#### Scenario: Task already claimed
- **WHEN** a worker calls `task_claim` for a task that is already `in_progress` with another owner
- **THEN** the system returns an error indicating the task is already claimed by another agent

#### Scenario: Task is blocked
- **WHEN** a worker calls `task_claim` for a task with status `blocked`
- **THEN** the system returns an error indicating the task has unsatisfied dependencies, listing the blocking tasks

### Requirement: Complete task with cascading unblock
The system SHALL provide a `task_complete` tool that marks a task as completed and triggers the event dispatcher to unblock dependent tasks, notify the lead, notify newly unblocked agents, and update cmux.

#### Scenario: Complete task that unblocks others
- **WHEN** a worker calls `task_complete` for task `1.1`, and task `2.1` depends only on `1.1`
- **THEN** the system marks `1.1` as `completed`, changes `2.1` from `blocked` to `pending`, triggers the event dispatcher to push notifications to the lead and to the agent assigned task `2.1`

#### Scenario: Complete final task
- **WHEN** a worker calls `task_complete` and this is the last incomplete task in the team
- **THEN** the system marks the task as `completed` and triggers the event dispatcher to notify the lead that all tasks are complete and the team is ready to converge

### Requirement: Fail task with notification
The system SHALL provide a `task_fail` tool that marks a task as failed with a reason and notifies the lead.

#### Scenario: Task failure
- **WHEN** a worker calls `task_fail` with task ID and reason `"compilation error in auth module"`
- **THEN** the system marks the task as `failed` with the reason, and triggers the event dispatcher to notify the lead with the failure details
