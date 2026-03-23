## ADDED Requirements

### Requirement: Send message to agent or topic
The system SHALL provide a `send_message` tool that delivers a message to a specific agent (`@agent-name`) or a topic (`#team`, `#conflict`). Messages SHALL be stored in SQLite and pushed to recipients via the OpenCode SDK.

#### Scenario: Direct message to agent
- **WHEN** agent `alice` calls `send_message` with recipient `@bob` and body `"Can you adjust your auth.rs changes?"`
- **THEN** the system stores the message in SQLite, and pushes it to `bob`'s OpenCode session via `POST /session/{id}/prompt`

#### Scenario: Broadcast to team topic
- **WHEN** any agent calls `send_message` with topic `#team` and body `"Merged branch teams/alice into main"`
- **THEN** the system stores the message and pushes it to all other active agents' sessions

#### Scenario: Send to offline agent
- **WHEN** an agent sends a message to a recipient that is crashed or killed
- **THEN** the system stores the message in SQLite (for potential catch-up) and returns a warning that the recipient is not currently active

### Requirement: Read inbox messages
The system SHALL provide a `read_inbox` tool that returns messages addressed to the calling agent, including both direct messages and topic messages for topics the agent is subscribed to.

#### Scenario: Read unread messages only
- **WHEN** a worker calls `read_inbox` with `unread_only: true`
- **THEN** the system returns all unread messages for the agent (direct + topic), marks them as read, and returns them sorted by creation time

#### Scenario: Read all messages
- **WHEN** a worker calls `read_inbox` without filters
- **THEN** the system returns all messages for the agent, both read and unread, sorted by creation time

### Requirement: Conflict negotiation message type
The system SHALL support a `conflict_negotiation` message type that includes structured conflict context (conflicting files, diffs, counterpart agent) alongside the text body.

#### Scenario: Conflict negotiation message
- **WHEN** the event dispatcher sends a conflict_negotiation message to agent `bob` about a conflict with `alice` on `auth.rs`
- **THEN** the message includes the message type `conflict_negotiation`, the conflicting file paths, a summary of alice's diff, and the text body explaining what needs to be resolved
