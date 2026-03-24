## ADDED Requirements

### Requirement: Create git worktree for agent
The system SHALL create a git worktree with a dedicated branch when an agent profile is created. The worktree SHALL be located at `<project>/.worktrees/<agent-name>/` with branch `teams/<agent-name>`.

#### Scenario: Worktree creation from HEAD
- **WHEN** an agent profile is created and no other agents have merged yet
- **THEN** the system creates a worktree at `<project>/.worktrees/<agent-name>/` with a new branch `teams/<agent-name>` based on the current HEAD of the main branch, and records the base commit in the worktrees table

#### Scenario: Worktree creation after merges
- **WHEN** an agent profile is created after other agents have already merged into main
- **THEN** the system creates the worktree with a branch based on the current HEAD of main (which includes prior merges), giving the new agent the benefit of completed work

### Requirement: Query worktree status
The system SHALL provide a `worktree_status` tool that returns the branch state, divergence from main, and list of modified files for a given agent's worktree.

#### Scenario: Worktree with divergence
- **WHEN** any agent calls `worktree_status` for an agent whose branch has diverged from main
- **THEN** the system returns the branch name, number of commits ahead of main, number of commits behind main, and list of files modified in the agent's branch

### Requirement: Merge agent branch to main
The system SHALL provide a `merge_to_main` tool that attempts to merge the calling agent's branch into the main branch. On success, it SHALL broadcast the merge to all agents. On conflict, it SHALL identify the counterpart agents and trigger conflict negotiation.

#### Scenario: Clean merge
- **WHEN** a worker calls `merge_to_main` and the merge completes without conflicts
- **THEN** the system merges the branch into main, updates the worktree status to `merged`, broadcasts a merge notification to `#team` with the list of changed files and commit range, and updates cmux sidebar

#### Scenario: Merge with conflicts
- **WHEN** a worker calls `merge_to_main` and the merge has conflicts
- **THEN** the system aborts the merge attempt, identifies which files conflict, queries `file_changes` to determine which other agent(s) touched those files, records the conflict in SQLite, returns the conflict details to the calling agent, and triggers the event dispatcher to notify the counterpart agent(s) and the lead

### Requirement: Get conflict details
The system SHALL provide a `get_conflict_details` tool that returns detailed information about a merge conflict including conflicting files, both sides of the conflict, and the counterpart agent(s).

#### Scenario: Retrieve conflict details
- **WHEN** an agent calls `get_conflict_details` after a failed merge
- **THEN** the system returns the list of conflicting files, the content from both sides (ours vs theirs), and the name(s) of the counterpart agent(s) whose changes caused the conflict

### Requirement: Get another agent's diff
The system SHALL provide a `get_agent_diff` tool that returns another agent's changes for a specific file or all files, enabling conflict negotiation.

#### Scenario: View counterpart changes
- **WHEN** agent `bob` calls `get_agent_diff` for agent `alice` on file `src/auth.rs`
- **THEN** the system returns the diff of `alice`'s changes to `src/auth.rs` relative to the common ancestor

### Requirement: Rebase from main
The system SHALL provide a `rebase_from_main` tool that rebases the calling agent's branch onto the current main branch.

#### Scenario: Clean rebase
- **WHEN** a worker calls `rebase_from_main` and the rebase completes without conflicts
- **THEN** the system rebases the branch onto main, updates the base commit in the worktrees table, and returns success

#### Scenario: Rebase with conflicts
- **WHEN** a worker calls `rebase_from_main` and the rebase has conflicts
- **THEN** the system aborts the rebase, returns the conflicting files, and the agent can decide how to proceed

### Requirement: Clean up worktree
The system SHALL remove an agent's worktree and optionally delete its branch when the team ends or the agent is removed.

#### Scenario: Cleanup after merge
- **WHEN** `team_end` is called and an agent's branch has been merged
- **THEN** the system removes the worktree directory and deletes the branch, updating the worktree status to `cleaned_up`

#### Scenario: Cleanup with preserved branch
- **WHEN** `team_end` is called and an agent's branch has NOT been merged (crash/kill)
- **THEN** the system removes the worktree directory but preserves the branch for potential manual recovery, updating the worktree status to `cleaned_up`
