## ADDED Requirements

### Requirement: Dual transport MCP server
The system SHALL serve MCP tools over both stdio (for the lead agent) and streamable HTTP (for worker agents) from a single process using a shared `ServerHandler`.

#### Scenario: Lead connects via stdio
- **WHEN** OpenCode spawns the MCP server as a local tool
- **THEN** the system serves MCP protocol over stdin/stdout, identifying the caller as the lead agent

#### Scenario: Worker connects via HTTP
- **WHEN** a worker's OpenCode instance connects to the HTTP endpoint with an `X-Agent-Profile` header
- **THEN** the system serves MCP protocol over the HTTP transport, identifying the caller by the profile ID in the header

### Requirement: Agent identification from transport
The system SHALL extract the calling agent's identity from the transport context: the stdio connection is always the lead, and HTTP connections identify the agent via the `X-Agent-Profile` header.

#### Scenario: Identify lead agent
- **WHEN** an MCP tool call arrives over the stdio transport
- **THEN** the system sets the caller context to the lead agent's profile

#### Scenario: Identify worker agent
- **WHEN** an MCP tool call arrives over HTTP with header `X-Agent-Profile: alice-profile-123`
- **THEN** the system sets the caller context to the profile with ID `alice-profile-123`

#### Scenario: Missing profile header
- **WHEN** an HTTP MCP request arrives without an `X-Agent-Profile` header
- **THEN** the system returns an error indicating agent identification is required

### Requirement: Shared state across transports
The system SHALL use a single SQLite database and shared application state across both transports, ensuring all agents see consistent data.

#### Scenario: Cross-transport consistency
- **WHEN** the lead creates a task dependency via stdio and a worker queries task_list via HTTP
- **THEN** the worker sees the dependency reflected in the task statuses

### Requirement: CLI argument parsing
The system SHALL accept command-line arguments for configuration: HTTP port, database path, project path, and log level.

#### Scenario: Start with custom port
- **WHEN** the user starts the server with `--port 8080`
- **THEN** the HTTP transport listens on port 8080

#### Scenario: Default configuration
- **WHEN** the user starts the server with no arguments
- **THEN** the system uses default values: port 3001, database in the project directory, project path from current directory

### Requirement: Register all MCP tools
The system SHALL register all team lifecycle, agent management, task engine, message bus, worktree, and bridge tools as MCP tools with proper JSON schemas for parameters and return types.

#### Scenario: Tool discovery
- **WHEN** an agent connects and requests the tool list
- **THEN** the system returns all registered tools with their names, descriptions, and parameter schemas
