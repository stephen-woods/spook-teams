use std::sync::Arc;

use anyhow::Result;
use axum::Router;use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    schemars, serve_server, tool, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::Config;
use crate::db::Db;
use crate::event::EventDispatcher;
use crate::opencode::PortAllocator;
use crate::spawner::Spawner;

// ── AppState ─────────────────────────────────────────────────────────────────

/// Shared application state across all transports.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<Config>,
    pub spawner: Arc<dyn Spawner>,
    pub dispatcher: Arc<EventDispatcher>,
    pub port_allocator: Arc<PortAllocator>,
    /// Active managed processes keyed by member_id
    pub processes: Arc<tokio::sync::RwLock<
        std::collections::HashMap<String, Arc<tokio::sync::Mutex<crate::opencode::ManagedProcess>>>,
    >>,
    /// HTTP cancellation token (for graceful shutdown)
    pub http_cancel: CancellationToken,
}

impl AppState {
    pub fn new(db: Arc<Db>, config: Arc<Config>, spawner: Arc<dyn Spawner>) -> Self {
        let dispatcher = Arc::new(EventDispatcher::new(db.clone(), spawner.clone()));
        let port_allocator = Arc::new(PortAllocator::new(config.agent_base_port));
        Self {
            db,
            config,
            spawner,
            dispatcher,
            port_allocator,
            processes: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            http_cancel: CancellationToken::new(),
        }
    }
}

// ── Caller context ────────────────────────────────────────────────────────────

/// Identifies the agent making a tool call.
#[derive(Debug, Clone)]
pub enum CallerContext {
    /// The lead agent connected via stdio.
    Lead,
    /// A worker agent connected via HTTP, identified by profile ID.
    Worker { profile_id: String },
}

impl CallerContext {
    pub fn is_lead(&self) -> bool {
        matches!(self, CallerContext::Lead)
    }

    pub fn profile_id(&self) -> Option<&str> {
        match self {
            CallerContext::Worker { profile_id } => Some(profile_id),
            CallerContext::Lead => None,
        }
    }
}

// ── Tool parameter types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamCreateParams {
    /// Name for this team session (e.g. "feature-auth")
    pub name: String,
    /// OpenSpec change name (e.g. "build-spook-teams")
    pub change_name: String,
    /// Path to the project root (defaults to current directory)
    pub project_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamStatusParams {
    /// Team ID to query (uses the active team if omitted)
    pub team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamEndParams {
    /// Team ID to end
    pub team_id: String,
    /// Whether to clean up worktrees (default true)
    pub cleanup_worktrees: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentProfileCreateParams {
    /// Team ID the agent belongs to
    pub team_id: String,
    /// Human-readable agent name (e.g. "alice")
    pub name: String,
    /// Task source IDs assigned to this agent (e.g. ["1.1", "1.2"])
    pub task_ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentSpawnParams {
    /// Team ID
    pub team_id: String,
    /// Member (profile) ID to spawn
    pub member_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentWhoamiParams {
    /// Team ID (required for worker identification)
    pub team_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentStatusParams {
    /// Team ID
    pub team_id: String,
    /// Member ID to query
    pub member_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentKillParams {
    /// Team ID
    pub team_id: String,
    /// Member ID to kill
    pub member_id: String,
    /// Optional reason for killing
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskListParams {
    /// Team ID
    pub team_id: String,
    /// Filter: "all", "mine", or "available"
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskSetDependencyParams {
    /// Team ID
    pub team_id: String,
    /// Task ID that depends on another task
    pub task_id: String,
    /// Task ID that must be completed first
    pub depends_on_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskClaimParams {
    /// Team ID
    pub team_id: String,
    /// Task source_id or UUID to claim
    pub task_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCompleteParams {
    /// Team ID
    pub team_id: String,
    /// Task source_id or UUID
    pub task_id: String,
    /// Optional summary of what was done
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskFailParams {
    /// Team ID
    pub team_id: String,
    /// Task source_id or UUID
    pub task_id: String,
    /// Reason for failure
    pub reason: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Team ID
    pub team_id: String,
    /// Recipient: "@agent-name" for direct, "#team" for broadcast, "#conflict" for conflict-related
    pub recipient: String,
    /// Message body
    pub body: String,
    /// Optional metadata (JSON string)
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadInboxParams {
    /// Team ID
    pub team_id: String,
    /// If true, only return unread messages (and mark them read)
    pub unread_only: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeStatusParams {
    /// Team ID
    pub team_id: String,
    /// Agent name (defaults to calling agent)
    pub agent_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MergeToMainParams {
    /// Team ID
    pub team_id: String,
    /// Optional commit message
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetConflictDetailsParams {
    /// Team ID
    pub team_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAgentDiffParams {
    /// Team ID
    pub team_id: String,
    /// Agent whose diff to retrieve
    pub agent_name: String,
    /// Optional file path to limit the diff
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RebaseFromMainParams {
    /// Team ID
    pub team_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorktreeCleanupParams {
    /// Team ID
    pub team_id: String,
    /// Member ID whose worktree to clean up
    pub member_id: String,
    /// Whether to delete the branch (default false)
    pub delete_branch: Option<bool>,
}

// ── ServerHandler implementation ──────────────────────────────────────────────

/// The MCP server handler — serves both stdio (lead) and HTTP (workers) from a single struct.
#[derive(Clone)]
pub struct SpookTeamsHandler {
    pub state: AppState,
    /// If Some, this instance serves a specific worker (identified by profile_id in HTTP header).
    /// If None, this instance serves via stdio (lead).
    pub caller: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl SpookTeamsHandler {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            caller: None,
            tool_router: Self::tool_router(),
        }
    }

    pub fn with_caller(state: AppState, caller: String) -> Self {
        Self {
            state,
            caller: Some(caller),
            tool_router: Self::tool_router(),
        }
    }

    pub fn caller_context(&self) -> CallerContext {
        match &self.caller {
            Some(id) => CallerContext::Worker {
                profile_id: id.clone(),
            },
            None => CallerContext::Lead,
        }
    }

    /// Resolve a task by either its source_id or UUID for a given team.
    fn resolve_task_id(
        &self,
        team_id: &str,
        task_id: &str,
    ) -> Result<String, rmcp::ErrorData> {
        // First try as UUID directly
        {
            let conn = self.state.db.readers.get()
                .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
            if let Ok(Some(task)) = crate::db::task::get(&conn, task_id) {
                if task.team_id == team_id {
                    return Ok(task.id);
                }
            }
            // Try as source_id
            if let Ok(Some(task)) = crate::db::task::get_by_source_id(&conn, team_id, task_id) {
                return Ok(task.id);
            }
        }
        Err(rmcp::ErrorData::invalid_params(
            format!("Task not found: {}", task_id),
            None,
        ))
    }
}

#[tool_router]
impl SpookTeamsHandler {
    // ── Team lifecycle ─────────────────────────────────────────────────────

    #[tool(description = "Create a new team session and import tasks from OpenSpec tasks.md")]
    async fn team_create(
        &self,
        Parameters(params): Parameters<TeamCreateParams>,
    ) -> String {
        match crate::team::team_create(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get comprehensive status of the team including agents, tasks, and conflicts")]
    async fn team_status(
        &self,
        Parameters(params): Parameters<TeamStatusParams>,
    ) -> String {
        match crate::team::team_status(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "End the team session, export task states, kill agents, and clean up worktrees")]
    async fn team_end(
        &self,
        Parameters(params): Parameters<TeamEndParams>,
    ) -> String {
        match crate::team::team_end(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ── Agent management ───────────────────────────────────────────────────

    #[tool(description = "Create an agent profile with worktree, branch, and config files")]
    async fn agent_profile_create(
        &self,
        Parameters(params): Parameters<AgentProfileCreateParams>,
    ) -> String {
        match crate::agent::agent_profile_create(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Spawn an OpenCode serve instance for an agent and set up cmux workspace")]
    async fn agent_spawn(
        &self,
        Parameters(params): Parameters<AgentSpawnParams>,
    ) -> String {
        match crate::agent::agent_spawn(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Return the calling agent's identity, assigned tasks, and team context")]
    async fn agent_whoami(
        &self,
        Parameters(params): Parameters<AgentWhoamiParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::agent::agent_whoami(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get health, current task, and progress for a specific agent")]
    async fn agent_status(
        &self,
        Parameters(params): Parameters<AgentStatusParams>,
    ) -> String {
        match crate::agent::agent_status(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Kill an agent process, preserve branch, and mark tasks failed")]
    async fn agent_kill(
        &self,
        Parameters(params): Parameters<AgentKillParams>,
    ) -> String {
        match crate::agent::agent_kill(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ── Task engine ────────────────────────────────────────────────────────

    #[tool(description = "List tasks with filter: 'all', 'mine', or 'available' (unblocked, unclaimed)")]
    async fn task_list(
        &self,
        Parameters(params): Parameters<TaskListParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::task::task_list(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Set a dependency between tasks (task_id depends on depends_on_id). Detects cycles.")]
    async fn task_set_dependency(
        &self,
        Parameters(params): Parameters<TaskSetDependencyParams>,
    ) -> String {
        match crate::task::task_set_dependency(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Atomically claim a pending task for the calling agent")]
    async fn task_claim(
        &self,
        Parameters(params): Parameters<TaskClaimParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::task::task_claim(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Mark a task as completed and trigger cascading unblock of dependent tasks")]
    async fn task_complete(
        &self,
        Parameters(params): Parameters<TaskCompleteParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::task::task_complete(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Mark a task as failed with a reason and notify the lead")]
    async fn task_fail(
        &self,
        Parameters(params): Parameters<TaskFailParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::task::task_fail(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ── Message bus ────────────────────────────────────────────────────────

    #[tool(description = "Send a message to an agent (@name) or topic (#team, #conflict)")]
    async fn send_message(
        &self,
        Parameters(params): Parameters<SendMessageParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::message::send_message(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Read inbox messages for the calling agent, optionally filtering to unread only")]
    async fn read_inbox(
        &self,
        Parameters(params): Parameters<ReadInboxParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::message::read_inbox(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ── Git worktree operations ────────────────────────────────────────────

    #[tool(description = "Get worktree status: branch, divergence from main, modified files")]
    async fn worktree_status(
        &self,
        Parameters(params): Parameters<WorktreeStatusParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::worktree::worktree_status(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Attempt to merge the calling agent's branch into main")]
    async fn merge_to_main(
        &self,
        Parameters(params): Parameters<MergeToMainParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::worktree::merge_to_main(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get detailed conflict information: files, both sides, counterpart agents")]
    async fn get_conflict_details(
        &self,
        Parameters(params): Parameters<GetConflictDetailsParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::worktree::get_conflict_details(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get another agent's diff relative to the common ancestor")]
    async fn get_agent_diff(
        &self,
        Parameters(params): Parameters<GetAgentDiffParams>,
    ) -> String {
        match crate::worktree::get_agent_diff(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Rebase the calling agent's branch onto current main")]
    async fn rebase_from_main(
        &self,
        Parameters(params): Parameters<RebaseFromMainParams>,
    ) -> String {
        let caller = self.caller_context();
        match crate::worktree::rebase_from_main(&self.state, params, caller).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Remove a worktree directory and optionally delete its branch")]
    async fn worktree_cleanup(
        &self,
        Parameters(params): Parameters<WorktreeCleanupParams>,
    ) -> String {
        match crate::worktree::worktree_cleanup(&self.state, params).await {
            Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| format!("Error serializing: {}", e)),
            Err(e) => format!("Error: {}", e),
        }
    }
}

impl ServerHandler for SpookTeamsHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "spook-teams: coordinate parallel OpenCode agents working on OpenSpec changes. \
                 Use team_create to start, agent_profile_create + agent_spawn to launch workers, \
                 then task_claim/task_complete to track work. Workers identify themselves via X-Agent-Profile header."
            )
    }
}

// ── Axum middleware to extract X-Agent-Profile ────────────────────────────────

/// Axum layer that reads `X-Agent-Profile` header and injects a
/// `SpookTeamsHandler` with the correct caller context.
fn make_http_service(state: AppState) -> StreamableHttpService<SpookTeamsHandler> {
    let config = StreamableHttpServerConfig {
        cancellation_token: state.http_cancel.clone(),
        ..Default::default()
    };

    StreamableHttpService::new(
        move || {
            // The service factory is called per request/session.
            // We return a handler with no caller set — caller is extracted from the header
            // in a wrapping axum layer below.
            Ok(SpookTeamsHandler::new(state.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

// ── run_server ────────────────────────────────────────────────────────────────

pub async fn run_server(config: Config, db: Arc<Db>) -> Result<()> {
    let config = Arc::new(config);
    let spawner: Arc<dyn Spawner> = Arc::from(crate::spawner::detect_spawner());
    let state = AppState::new(db, config.clone(), spawner);

    let port = config.port;
    let http_cancel = state.http_cancel.clone();

    // ── Stdio server (lead agent) ─────────────────────────────────────────
    let stdio_handler = SpookTeamsHandler::new(state.clone());
    let stdio_task = tokio::spawn(async move {
        let transport = rmcp::transport::io::stdio();
        match serve_server(stdio_handler, transport).await {
            Ok(service) => {
                info!("Stdio MCP server started (lead connection)");
                if let Err(e) = service.waiting().await {
                    warn!("Stdio server error: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to start stdio server: {}", e);
            }
        }
    });

    // ── HTTP server (worker agents) ───────────────────────────────────────
    // Mount StreamableHttpService directly as the axum fallback service.
    // The service factory reads the X-Agent-Profile header to set caller context.
    // Worker identification happens inside each tool handler via CallerContext.
    let http_app = axum::Router::new().fallback_service(
        tower::ServiceBuilder::new()
            .service(make_http_service(state.clone()))
    );

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!(port, "HTTP MCP server listening for worker agents");

    let http_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, http_app)
            .with_graceful_shutdown(async move { http_cancel.cancelled().await })
            .await
        {
            warn!("HTTP server error: {}", e);
        }
    });

    // Wait for either transport to finish
    tokio::select! {
        _ = stdio_task => {
            info!("Stdio server exited, shutting down HTTP server");
            state.http_cancel.cancel();
        }
        _ = http_task => {
            info!("HTTP server exited");
        }
    }

    Ok(())
}
