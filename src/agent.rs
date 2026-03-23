use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::db::{self, member::MemberRole, member::MemberStatus, task::TaskFilter, task::TaskStatus};
use crate::opencode;
use crate::server::{
    AgentKillParams, AgentProfileCreateParams, AgentSpawnParams, AgentStatusParams,
    AgentWhoamiParams, AppState, CallerContext,
};
use crate::spawner::{SplitDirection, StatusUpdate, WorkspaceId};
use crate::worktree::create_worktree_for_agent;

// ── Return types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentProfileCreateResult {
    pub member_id: String,
    pub name: String,
    pub worktree_path: String,
    pub branch: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentSpawnResult {
    pub member_id: String,
    pub name: String,
    pub port: u16,
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentWhoamiResult {
    pub member_id: String,
    pub name: String,
    pub role: String,
    pub team_id: String,
    pub team_name: String,
    pub change_name: String,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub assigned_tasks: Vec<TaskSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentStatusResult {
    pub member_id: String,
    pub name: String,
    pub status: String,
    pub current_task: Option<TaskSummary>,
    pub completed_count: usize,
    pub total_count: usize,
    pub worktree_branch: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentKillResult {
    pub member_id: String,
    pub name: String,
    pub tasks_failed: usize,
    pub message: String,
}

// ── agent_profile_create (12.1) ───────────────────────────────────────────────

pub async fn agent_profile_create(
    state: &AppState,
    params: AgentProfileCreateParams,
) -> Result<AgentProfileCreateResult> {
    // Check team exists
    let (team, project_path) = {
        let conn = state.db.readers.get()?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found: {}", params.team_id))?;
        let project_path = team.project_path.clone();
        (team, project_path)
    };

    // Check for duplicate name
    {
        let conn = state.db.readers.get()?;
        if db::member::get_by_name(&conn, &params.team_id, &params.name)?.is_some() {
            anyhow::bail!("Agent name '{}' is already taken in this team", params.name);
        }
    }

    // Create the member record
    let member = {
        let conn = state.db.writer.lock().unwrap();
        db::member::create(&conn, &params.team_id, &params.name, MemberRole::Worker)?
    };

    // Create git worktree + branch
    let (worktree_path, branch, base_commit) =
        create_worktree_for_agent(&project_path, &params.name).await?;

    // Record worktree in DB
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let wt = {
        let conn = state.db.writer.lock().unwrap();
        db::worktree::create(
            &conn,
            &params.team_id,
            &member.id,
            &worktree_path_str,
            &branch,
            Some(&base_commit),
        )?
    };

    // Link worktree to member
    {
        let conn = state.db.writer.lock().unwrap();
        db::member::update_worktree(&conn, &member.id, &wt.id)?;
    }

    // Write opencode.json into the worktree
    write_opencode_config(&worktree_path, &member.id, state.config.port).await?;

    // Write worker.md agent prompt into the worktree
    write_worker_md(&worktree_path, &params.name, &team.change_name).await?;

    // Assign tasks to this member (look up by source_id, set owner without changing status)
    if !params.task_ids.is_empty() {
        let conn = state.db.writer.lock().unwrap();
        for source_id in &params.task_ids {
            if let Ok(Some(task)) = db::task::get_by_source_id(&conn, &params.team_id, source_id) {
                // Set owner but keep status as pending (claim will flip to in_progress later)
                let _ = conn.execute(
                    "UPDATE tasks SET owner_id = ?1, updated_at = datetime('now') WHERE id = ?2",
                    rusqlite::params![member.id, task.id],
                );
            }
        }
    }

    info!(
        member_id = %member.id,
        name = %params.name,
        branch = %branch,
        worktree = %worktree_path_str,
        "Agent profile created"
    );

    Ok(AgentProfileCreateResult {
        member_id: member.id,
        name: params.name,
        worktree_path: worktree_path_str,
        branch,
        message: format!(
            "Agent profile created. Worktree at '{}'. Use agent_spawn to start the agent.",
            worktree_path.display()
        ),
    })
}

// ── agent_spawn (12.2) ────────────────────────────────────────────────────────

pub async fn agent_spawn(state: &AppState, params: AgentSpawnParams) -> Result<AgentSpawnResult> {
    // Fetch member + team info
    let (member, team, worktree_path, is_respawn) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &params.member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found: {}", params.member_id))?;
        if member.team_id != params.team_id {
            anyhow::bail!("Member does not belong to team {}", params.team_id);
        }
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found: {}", params.team_id))?;
        let wt = db::worktree::get_by_member(&conn, &params.member_id)?
            .ok_or_else(|| anyhow::anyhow!("No worktree found for member"))?;
        let is_respawn = member.status == MemberStatus::Crashed;
        (member, team, std::path::PathBuf::from(wt.path), is_respawn)
    };

    // Allocate port
    let port = state.port_allocator.allocate()?;

    // Spawn opencode serve
    let managed = opencode::spawn_serve(port, &worktree_path).await?;
    let client = opencode::OpenCodeClient::new(port);

    // Health probe
    client.wait_healthy(30).await.map_err(|e| {
        state.port_allocator.release(port);
        e
    })?;

    // Create session
    let session = client.create_session("worker").await.map_err(|e| {
        state.port_allocator.release(port);
        e
    })?;

    // Build initial prompt
    let initial_prompt = if is_respawn {
        build_respawn_prompt(&member.name, &team.change_name, state).await
    } else {
        build_initial_prompt(&member.name, &team.change_name, &params.team_id, state).await
    };

    // Push initial prompt
    client
        .push_prompt(&session.id, &initial_prompt)
        .await
        .map_err(|e| {
            state.port_allocator.release(port);
            e
        })?;

    // Subscribe SSE event stream
    let (sse_tx, _) = tokio::sync::broadcast::channel(64);
    opencode::subscribe_sse(client.base_url.clone(), member.id.clone(), sse_tx).await;

    // Store process handle
    {
        let mut processes = state.processes.write().await;
        processes.insert(
            member.id.clone(),
            Arc::new(tokio::sync::Mutex::new(managed)),
        );
    }

    // Update member record
    {
        let conn = state.db.writer.lock().unwrap();
        db::member::update_session(&conn, &member.id, &session.id, port)?;
        db::member::update_status(&conn, &member.id, MemberStatus::Active)?;
    }

    // Spawn process monitor task
    spawn_monitor_task(state, member.id.clone(), params.team_id.clone(), port);

    // Set up cmux workspace (best effort)
    let workspace_name = format!("{}-{}", params.team_id, member.name);
    match state
        .spawner
        .create_workspace(&workspace_name, &worktree_path)
        .await
    {
        Ok(workspace) => {
            // Split pane for shell
            let _ = state
                .spawner
                .create_split(&workspace, SplitDirection::Right)
                .await;

            // Send opencode attach command in left pane
            let attach_cmd = format!("opencode attach --port {}\n", port);
            if let Ok(left) = state
                .spawner
                .create_split(&workspace, SplitDirection::Right)
                .await
            {
                let _ = state.spawner.send_keys(&left, &attach_cmd).await;
            }

            // Set sidebar status
            let _ = state
                .spawner
                .set_status(
                    &workspace,
                    &StatusUpdate {
                        text: "Working".to_string(),
                        icon: Some("⚙".to_string()),
                        color: Some("blue".to_string()),
                    },
                )
                .await;

            let _ = state
                .spawner
                .set_progress(&workspace, 0.0, "Starting...")
                .await;
        }
        Err(e) => {
            warn!(member_id = %member.id, "Failed to create cmux workspace: {}", e);
        }
    }

    info!(
        member_id = %member.id,
        name = %member.name,
        port,
        session_id = %session.id,
        "Agent spawned"
    );

    Ok(AgentSpawnResult {
        member_id: member.id.clone(),
        name: member.name.clone(),
        port,
        session_id: session.id,
        message: format!(
            "Agent '{}' spawned on port {}. Session started{}.",
            member.name,
            port,
            if is_respawn { " (respawn)" } else { "" }
        ),
    })
}

// ── agent_whoami (12.3) ───────────────────────────────────────────────────────

pub async fn agent_whoami(
    state: &AppState,
    params: AgentWhoamiParams,
    caller: CallerContext,
) -> Result<AgentWhoamiResult> {
    let member_id = match caller.profile_id() {
        Some(id) => id.to_string(),
        None => {
            // Lead: return the lead member record
            let conn = state.db.readers.get()?;
            let members = db::member::list_by_team(&conn, &params.team_id)?;
            members
                .into_iter()
                .find(|m| m.role == MemberRole::Lead)
                .ok_or_else(|| anyhow::anyhow!("Lead member not found"))?
                .id
        }
    };

    let (member, team, worktree) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found"))?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        let worktree = db::worktree::get_by_member(&conn, &member_id)?;
        (member, team, worktree)
    };

    // Get assigned tasks
    let assigned_tasks = {
        let conn = state.db.readers.get()?;
        db::task::list(&conn, &params.team_id, TaskFilter::Mine(member_id.clone()))?
            .into_iter()
            .map(|t| TaskSummary {
                id: t.id,
                source_id: t.source_id,
                title: t.title,
                status: t.status.as_str().to_string(),
            })
            .collect()
    };

    Ok(AgentWhoamiResult {
        member_id: member.id,
        name: member.name,
        role: member.role.as_str().to_string(),
        team_id: team.id,
        team_name: team.name,
        change_name: team.change_name,
        worktree_path: worktree.as_ref().map(|wt| wt.path.clone()),
        branch: worktree.map(|wt| wt.branch),
        assigned_tasks,
    })
}

// ── agent_status (12.4) ───────────────────────────────────────────────────────

pub async fn agent_status(
    state: &AppState,
    params: AgentStatusParams,
) -> Result<AgentStatusResult> {
    let (member, worktree) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &params.member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found: {}", params.member_id))?;
        if member.team_id != params.team_id {
            anyhow::bail!("Member does not belong to team {}", params.team_id);
        }
        let worktree = db::worktree::get_by_member(&conn, &params.member_id)?;
        (member, worktree)
    };

    let tasks = {
        let conn = state.db.readers.get()?;
        db::task::list(&conn, &params.team_id, TaskFilter::Mine(params.member_id.clone()))?
    };

    let current_task = tasks
        .iter()
        .find(|t| t.status == TaskStatus::InProgress)
        .map(|t| TaskSummary {
            id: t.id.clone(),
            source_id: t.source_id.clone(),
            title: t.title.clone(),
            status: t.status.as_str().to_string(),
        });

    let completed_count = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();

    Ok(AgentStatusResult {
        member_id: member.id,
        name: member.name,
        status: member.status.as_str().to_string(),
        current_task,
        completed_count,
        total_count: tasks.len(),
        worktree_branch: worktree.map(|wt| wt.branch),
        port: member.port,
    })
}

// ── agent_kill (12.5) ─────────────────────────────────────────────────────────

pub async fn agent_kill(state: &AppState, params: AgentKillParams) -> Result<AgentKillResult> {
    let (member, project_path) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &params.member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found: {}", params.member_id))?;
        if member.team_id != params.team_id {
            anyhow::bail!("Member does not belong to team {}", params.team_id);
        }
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        (member, team.project_path)
    };

    // Kill the process
    {
        let processes = state.processes.read().await;
        if let Some(proc) = processes.get(&params.member_id) {
            let mut proc = proc.lock().await;
            if let Err(e) = proc.kill().await {
                warn!(member_id = %params.member_id, "Failed to kill process: {}", e);
            }
            if let Some(port) = member.port {
                state.port_allocator.release(port);
            }
        }
    }
    // Remove from process map
    {
        let mut processes = state.processes.write().await;
        processes.remove(&params.member_id);
    }

    // Mark in-progress tasks as failed
    let in_progress_tasks: Vec<_> = {
        let conn = state.db.readers.get()?;
        db::task::list(&conn, &params.team_id, TaskFilter::Mine(params.member_id.clone()))?
            .into_iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .collect()
    };

    let tasks_failed = in_progress_tasks.len();
    let reason = params
        .reason
        .as_deref()
        .unwrap_or("agent_killed")
        .to_string();

    for task in &in_progress_tasks {
        let conn = state.db.writer.lock().unwrap();
        let _ = db::task::update_status(
            &conn,
            &task.id,
            db::task::TaskStatus::Failed,
        );
    }

    // Record kill event
    {
        let conn = state.db.writer.lock().unwrap();
        let payload = serde_json::json!({
            "reason": reason,
            "tasks_failed": tasks_failed,
        });
        let _ = db::file_changes::insert_agent_event(
            &conn,
            &params.team_id,
            &params.member_id,
            "agent_killed",
            Some(&payload.to_string()),
        );
    }

    // Update member status
    {
        let conn = state.db.writer.lock().unwrap();
        db::member::update_status(&conn, &params.member_id, MemberStatus::Killed)?;
    }

    // Update cmux sidebar (best effort)
    let workspace_name = format!("{}-{}", params.team_id, member.name);
    let ws = WorkspaceId(workspace_name);
    let _ = state
        .spawner
        .set_status(
            &ws,
            &StatusUpdate {
                text: "Killed".to_string(),
                icon: Some("✗".to_string()),
                color: Some("red".to_string()),
            },
        )
        .await;

    info!(
        member_id = %params.member_id,
        name = %member.name,
        tasks_failed,
        "Agent killed"
    );

    Ok(AgentKillResult {
        member_id: params.member_id,
        name: member.name,
        tasks_failed,
        message: format!(
            "Agent killed. {} in-progress tasks marked failed.",
            tasks_failed
        ),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Write opencode.json into the worktree to configure the worker's MCP connection.
async fn write_opencode_config(
    worktree_path: &std::path::Path,
    member_id: &str,
    server_port: u16,
) -> Result<()> {
    let config = serde_json::json!({
        "mcp": {
            "spook-teams": {
                "type": "http",
                "url": format!("http://127.0.0.1:{}/mcp", server_port),
                "headers": {
                    "X-Agent-Profile": member_id
                }
            }
        }
    });
    let config_path = worktree_path.join("opencode.json");
    tokio::fs::write(&config_path, serde_json::to_string_pretty(&config)?).await?;
    Ok(())
}

/// Write the worker agent system prompt into `.opencode/agents/worker.md`.
async fn write_worker_md(
    worktree_path: &std::path::Path,
    agent_name: &str,
    change_name: &str,
) -> Result<()> {
    let agents_dir = worktree_path.join(".opencode").join("agents");
    tokio::fs::create_dir_all(&agents_dir).await?;

    let content = format!(
        r#"# Worker Agent: {name}

You are a worker agent in the spook-teams parallel execution system, working on the OpenSpec change `{change}`.

## Your Role
- You work autonomously on assigned tasks from your task list
- You use the spook-teams MCP tools to coordinate with the lead and other workers
- You communicate via `send_message` / `read_inbox` for coordination

## Startup Checklist
1. Call `agent_whoami` to get your identity, assigned tasks, and team context
2. Call `task_list` with filter `mine` to see your assigned tasks
3. Pick the first available (unblocked) task and call `task_claim` to claim it
4. Implement the task, then call `task_complete` with a summary
5. Repeat until all your tasks are complete

## Key Rules
- Always claim a task before starting work on it
- If a task fails, call `task_fail` with the reason
- For merge conflicts, call `merge_to_main` first; if it fails, coordinate with `send_message` to the conflicting agent
- Use `read_inbox` to check for messages from the lead or other agents
- When all tasks are complete, call `send_message` to `#team` to announce completion

## MCP Tools Available
- `agent_whoami` — get your profile and assigned tasks
- `task_list` — list tasks (filter: mine/all/available)
- `task_claim` / `task_complete` / `task_fail` — manage task lifecycle
- `send_message` / `read_inbox` — communicate with team
- `worktree_status` — check your branch state
- `merge_to_main` / `get_conflict_details` / `rebase_from_main` — git operations
"#,
        name = agent_name,
        change = change_name,
    );

    tokio::fs::write(agents_dir.join("worker.md"), content).await?;
    Ok(())
}

/// Build the initial prompt for a freshly spawned agent.
async fn build_initial_prompt(
    agent_name: &str,
    change_name: &str,
    team_id: &str,
    state: &AppState,
) -> String {
    // Fetch the proposal for context
    let proposal_snippet = get_proposal_snippet(change_name, state).await;

    format!(
        r#"You are worker agent '{name}' working on OpenSpec change '{change}' (team: {team_id}).

{proposal}

Start by calling `agent_whoami` to get your full context and assigned tasks, then begin working.
"#,
        name = agent_name,
        change = change_name,
        team_id = team_id,
        proposal = proposal_snippet,
    )
}

/// Build an initial prompt for a respawned (crashed) agent.
async fn build_respawn_prompt(agent_name: &str, change_name: &str, state: &AppState) -> String {
    let _ = state; // may use state for crash context in the future
    format!(
        r#"You are worker agent '{name}' working on OpenSpec change '{change}'.
You are being respawned after a crash. Your previous work (branch and worktree) is preserved.

Start by calling `agent_whoami` to see your status and pick up from where you left off.
Call `task_list mine` to see which tasks are still pending and continue working.
"#,
        name = agent_name,
        change = change_name,
    )
}

async fn get_proposal_snippet(change_name: &str, state: &AppState) -> String {
    let proposal_path = std::path::PathBuf::from(state.config.project_path.as_path())
        .join("openspec")
        .join("changes")
        .join(change_name)
        .join("proposal.md");

    match tokio::fs::read_to_string(&proposal_path).await {
        Ok(content) => {
            // Return first 500 chars as context
            let snippet: String = content.chars().take(500).collect();
            format!("## Change Context\n{}", snippet)
        }
        Err(_) => String::new(),
    }
}

/// Spawn a background task that monitors the agent process and triggers
/// the crash dispatcher if it exits unexpectedly.
fn spawn_monitor_task(state: &AppState, member_id: String, team_id: String, port: u16) {
    let processes = state.processes.clone();
    let db = state.db.clone();
    let dispatcher = state.dispatcher.clone();
    let port_allocator = state.port_allocator.clone();
    let spawner = state.spawner.clone();

    tokio::spawn(async move {
        // Wait for the process to exit
        let status = {
            let procs = processes.read().await;
            if let Some(proc) = procs.get(&member_id) {
                let mut p = proc.lock().await;
                p.wait().await.ok()
            } else {
                return;
            }
        };

        let exited_normally = status.map(|s| s.success()).unwrap_or(false);

        // Check if the member is still marked as active (unexpected exit)
        let is_active = {
            if let Ok(conn) = db.readers.get() {
                db::member::get(&conn, &member_id)
                    .ok()
                    .flatten()
                    .map(|m| m.status == MemberStatus::Active)
                    .unwrap_or(false)
            } else {
                false
            }
        };

        if !exited_normally && is_active {
            warn!(member_id, "Agent process crashed unexpectedly");

            // Capture cmux screen context
            let workspace_name = {
                let conn = db.readers.get().ok();
                conn.and_then(|c| db::member::get(&c, &member_id).ok().flatten())
                    .map(|m| format!("{}-{}", team_id, m.name))
                    .unwrap_or_default()
            };
            let screen_context = spawner
                .read_screen(&WorkspaceId(workspace_name.clone()))
                .await
                .unwrap_or_default();

            // Mark in-progress tasks as failed
            let in_progress_tasks: Vec<_> = {
                let conn = db.readers.get().ok();
                conn.and_then(|c| {
                    db::task::list(&c, &team_id, TaskFilter::Mine(member_id.clone())).ok()
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t.status == TaskStatus::InProgress)
                .collect()
            };

            for task in &in_progress_tasks {
                if let Ok(conn) = db.writer.lock() {
                    let _ = db::task::update_status(
                        &conn,
                        &task.id,
                        TaskStatus::Failed,
                    );
                }
            }

            // Record crash event
            {
                let payload = serde_json::json!({
                    "screen_context": screen_context,
                    "tasks_failed": in_progress_tasks.len(),
                });
                if let Ok(conn) = db.writer.lock() {
                    let _ = db::file_changes::insert_agent_event(
                        &conn,
                        &team_id,
                        &member_id,
                        "agent_crashed",
                        Some(&payload.to_string()),
                    );
                }
            }

            // Update member status
            if let Ok(conn) = db.writer.lock() {
                let _ = db::member::update_status(&conn, &member_id, MemberStatus::Crashed);
            }

            // Update cmux sidebar
            let _ = spawner
                .set_status(
                    &WorkspaceId(workspace_name),
                    &StatusUpdate {
                        text: "Crashed".to_string(),
                        icon: Some("✗".to_string()),
                        color: Some("red".to_string()),
                    },
                )
                .await;

            // Trigger crash dispatcher
            let _ = dispatcher
                .on_crash(&team_id, &member_id, None, "agent_crashed")
                .await;
        }

        // Release port
        port_allocator.release(port);

        // Remove from process map
        let mut procs = processes.write().await;
        procs.remove(&member_id);
    });
}
