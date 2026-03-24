## ADDED Requirements

### Requirement: Import tasks from tasks.md
The system SHALL parse an OpenSpec `tasks.md` file and create corresponding task records in SQLite. Each numbered section becomes a task group and each checkbox item becomes an individual task with a `source_id` derived from the section and item numbering.

#### Scenario: Import standard tasks.md
- **WHEN** `team_create` is called and the change has a `tasks.md` with sections and checkbox items
- **THEN** the system parses each section heading and checkbox item, creates task records with `source_id` (e.g., `"1.1"`, `"2.3"`), preserves titles and descriptions, sets all tasks to status `pending`, and returns the count of imported tasks

#### Scenario: Import tasks.md with pre-checked items
- **WHEN** `tasks.md` contains items marked as `[x]` (already completed)
- **THEN** the system imports those tasks with status `completed` and does not assign them to any agent

### Requirement: Export task state to tasks.md
The system SHALL provide an `export_tasks` tool that writes the current SQLite task state back to `tasks.md`, updating checkbox states to reflect completion status.

#### Scenario: Export with mixed states
- **WHEN** the lead calls `export_tasks` and tasks have mixed statuses
- **THEN** the system writes `tasks.md` with `[x]` for completed tasks and `[ ]` for all other statuses, preserving the original section structure and task descriptions

### Requirement: Reimport tasks with diff reconciliation
The system SHALL provide a `reimport_tasks` tool that re-reads `tasks.md`, diffs it against the current SQLite state, and returns the changes for lead agent review before applying.

#### Scenario: New tasks added to tasks.md
- **WHEN** the lead calls `reimport_tasks` and `tasks.md` has new tasks that don't exist in SQLite
- **THEN** the system returns a diff showing the added tasks and their proposed status (`pending`), without applying changes until the lead confirms

#### Scenario: Tasks removed from tasks.md
- **WHEN** the lead calls `reimport_tasks` and `tasks.md` is missing tasks that exist in SQLite
- **THEN** the system returns a diff showing the removed tasks and their current status, flagging any that are `in_progress` as requiring attention before removal

#### Scenario: Task descriptions modified
- **WHEN** the lead calls `reimport_tasks` and task descriptions have changed in `tasks.md`
- **THEN** the system returns a diff showing the old and new descriptions, without applying changes until the lead confirms
