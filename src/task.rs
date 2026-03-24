use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::db::{self, task::{Task, TaskFilter, TaskStatus}};
use crate::server::{
    AppState, CallerContext, TaskClaimParams, TaskCompleteParams, TaskFailParams,
    TaskListParams, TaskSetDependencyParams,
};

// ── Return types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskListResult {
    pub tasks: Vec<TaskSummary>,
    pub count: usize,
    pub filter: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub status: String,
    pub owner_id: Option<String>,
    pub section: Option<String>,
}

impl From<&Task> for TaskSummary {
    fn from(t: &Task) -> Self {
        TaskSummary {
            id: t.id.clone(),
            source_id: t.source_id.clone(),
            title: t.title.clone(),
            status: t.status.as_str().to_string(),
            owner_id: t.owner_id.clone(),
            section: t.section.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskDependencyResult {
    pub task_id: String,
    pub depends_on_id: String,
    pub blocked: bool,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskClaimResult {
    pub task: TaskSummary,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskCompleteResult {
    pub task_id: String,
    pub newly_unblocked: Vec<String>,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskFailResult {
    pub task_id: String,
    pub reason: String,
    pub message: String,
}

// ── task_list (7.1) ───────────────────────────────────────────────────────────

pub async fn task_list(
    state: &AppState,
    params: TaskListParams,
    caller: CallerContext,
) -> Result<TaskListResult> {
    let filter_str = params.filter.as_deref().unwrap_or("all");

    let filter = match filter_str {
        "mine" => {
            // Resolve caller's member ID for scoping
            let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
            TaskFilter::Mine(member_id)
        }
        "available" => TaskFilter::Available,
        _ => TaskFilter::All,
    };

    let tasks = {
        let conn = state.db.readers.get()?;
        db::task::list(&conn, &params.team_id, filter)?
    };

    let summaries: Vec<TaskSummary> = tasks.iter().map(TaskSummary::from).collect();
    let count = summaries.len();

    Ok(TaskListResult {
        count,
        tasks: summaries,
        filter: filter_str.to_string(),
    })
}

// ── task_set_dependency (7.2) ─────────────────────────────────────────────────

pub async fn task_set_dependency(
    state: &AppState,
    params: TaskSetDependencyParams,
) -> Result<TaskDependencyResult> {
    // Resolve task IDs (by source_id or UUID)
    let task_uuid = resolve_task_id(state, &params.team_id, &params.task_id)?;
    let dep_uuid = resolve_task_id(state, &params.team_id, &params.depends_on_id)?;

    // Add dependency with cycle detection
    {
        let conn = state.db.writer.lock().unwrap();
        db::task_dep::add_dependency(&conn, &task_uuid, &dep_uuid)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }

    // Check if dependency is satisfied (dep is completed)
    let dep_completed = {
        let conn = state.db.readers.get()?;
        db::task::get(&conn, &dep_uuid)?
            .map(|t| t.status == TaskStatus::Completed)
            .unwrap_or(false)
    };

    // If not completed, block the task
    let blocked = if !dep_completed {
        let conn = state.db.writer.lock().unwrap();
        db::task::update_status(&conn, &task_uuid, TaskStatus::Blocked)?;
        true
    } else {
        false
    };

    Ok(TaskDependencyResult {
        task_id: task_uuid,
        depends_on_id: dep_uuid,
        blocked,
        message: if blocked {
            format!(
                "Dependency added. Task '{}' is now blocked on '{}'.",
                params.task_id, params.depends_on_id
            )
        } else {
            format!(
                "Dependency added. Task '{}' is already satisfied (depends_on is complete).",
                params.task_id
            )
        },
    })
}

// ── task_claim (7.3) ──────────────────────────────────────────────────────────

pub async fn task_claim(
    state: &AppState,
    params: TaskClaimParams,
    caller: CallerContext,
) -> Result<TaskClaimResult> {
    let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
    let task_uuid = resolve_task_id(state, &params.team_id, &params.task_id)?;

    // Atomic claim using BEGIN IMMEDIATE
    let task = {
        let conn = state.db.writer.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE")?;

        // Re-read under the writer lock
        let task_result = db::task::get(&conn, &task_uuid)?;

        let task = match task_result {
            None => {
                conn.execute_batch("ROLLBACK")?;
                anyhow::bail!("Task not found: {}", params.task_id);
            }
            Some(t) => t,
        };

        // Validate state
        match task.status {
            TaskStatus::InProgress => {
                conn.execute_batch("ROLLBACK")?;
                let owner = task.owner_id.clone().unwrap_or_default();
                anyhow::bail!("Task is already claimed by '{}'.", owner);
            }
            TaskStatus::Blocked => {
                conn.execute_batch("ROLLBACK")?;
                anyhow::bail!(
                    "Task is blocked by unsatisfied dependencies. Use task_list with filter='available' to find claimable tasks."
                );
            }
            TaskStatus::Completed => {
                conn.execute_batch("ROLLBACK")?;
                anyhow::bail!("Task is already completed.");
            }
            TaskStatus::Failed => {
                conn.execute_batch("ROLLBACK")?;
                anyhow::bail!("Task is marked failed.");
            }
            TaskStatus::Pending | TaskStatus::Cancelled => {}
        }

        db::task::claim(&conn, &task_uuid, &member_id)?;
        conn.execute_batch("COMMIT")?;
        task
    };

    info!(task_id = %task_uuid, member_id = %member_id, "Task claimed");

    Ok(TaskClaimResult {
        message: format!("Task '{}' claimed successfully.", task.title),
        task: TaskSummary {
            id: task_uuid,
            source_id: task.source_id,
            title: task.title,
            status: TaskStatus::InProgress.as_str().to_string(),
            owner_id: Some(member_id),
            section: task.section,
        },
    })
}

// ── task_complete (7.4) ───────────────────────────────────────────────────────

pub async fn task_complete(
    state: &AppState,
    params: TaskCompleteParams,
    caller: CallerContext,
) -> Result<TaskCompleteResult> {
    let task_uuid = resolve_task_id(state, &params.team_id, &params.task_id)?;

    // Get task before marking complete
    let task = {
        let conn = state.db.readers.get()?;
        db::task::get(&conn, &task_uuid)?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", params.task_id))?
    };

    // Mark as completed
    {
        let conn = state.db.writer.lock().unwrap();
        db::task::update_status(&conn, &task_uuid, TaskStatus::Completed)?;
    }

    // Resolve caller name for dispatcher
    let agent_name = resolve_caller_name(state, &params.team_id, &caller);

    // Fire event dispatcher
    let newly_unblocked = {
        let dispatcher = state.dispatcher.clone();
        let team_id = params.team_id.clone();
        let task_id = task_uuid.clone();
        let agent = agent_name.clone();
        dispatcher.on_task_complete(&team_id, &task_id, &agent).await?;

        // Get newly unblocked task titles for response
        let conn = state.db.readers.get()?;
        db::task_dep::compute_newly_unblocked(&conn, &task_uuid)?
            .into_iter()
            .filter_map(|id| {
                db::task::get(&conn, &id).ok()?.map(|t| t.source_id)
            })
            .collect::<Vec<_>>()
    };

    info!(task_id = %task_uuid, agent = %agent_name, "Task completed");

    Ok(TaskCompleteResult {
        task_id: task_uuid,
        newly_unblocked: newly_unblocked.clone(),
        message: format!(
            "Task '{}' marked complete.{}",
            task.title,
            if newly_unblocked.is_empty() {
                String::new()
            } else {
                format!(" Unblocked: {}.", newly_unblocked.join(", "))
            }
        ),
    })
}

// ── task_fail (7.5) ───────────────────────────────────────────────────────────

pub async fn task_fail(
    state: &AppState,
    params: TaskFailParams,
    caller: CallerContext,
) -> Result<TaskFailResult> {
    let task_uuid = resolve_task_id(state, &params.team_id, &params.task_id)?;

    let task = {
        let conn = state.db.readers.get()?;
        db::task::get(&conn, &task_uuid)?
            .ok_or_else(|| anyhow::anyhow!("Task not found: {}", params.task_id))?
    };

    {
        let conn = state.db.writer.lock().unwrap();
        db::task::update_status(&conn, &task_uuid, TaskStatus::Failed)?;
    }

    let agent_name = resolve_caller_name(state, &params.team_id, &caller);

    state
        .dispatcher
        .on_task_fail(&params.team_id, &task_uuid, &agent_name, &params.reason)
        .await?;

    info!(task_id = %task_uuid, reason = %params.reason, "Task failed");

    Ok(TaskFailResult {
        task_id: task_uuid,
        reason: params.reason.clone(),
        message: format!("Task '{}' marked failed: {}", task.title, params.reason),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn resolve_task_id(state: &AppState, team_id: &str, task_id: &str) -> Result<String> {
    let conn = state.db.readers.get()?;
    // Try UUID directly
    if let Ok(Some(task)) = db::task::get(&conn, task_id) {
        if task.team_id == team_id {
            return Ok(task.id);
        }
    }
    // Try source_id
    if let Ok(Some(task)) = db::task::get_by_source_id(&conn, team_id, task_id) {
        return Ok(task.id);
    }
    anyhow::bail!("Task not found: {}", task_id)
}

pub fn resolve_caller_member_id(
    state: &AppState,
    team_id: &str,
    caller: &CallerContext,
) -> Result<String> {
    match caller {
        CallerContext::Lead => {
            let conn = state.db.readers.get()?;
            let members = db::member::list_by_team(&conn, team_id)?;
            members
                .into_iter()
                .find(|m| m.role == db::member::MemberRole::Lead)
                .map(|m| m.id)
                .ok_or_else(|| anyhow::anyhow!("Lead member not found for team {}", team_id))
        }
        CallerContext::Worker { profile_id } => Ok(profile_id.clone()),
    }
}

pub fn resolve_caller_name(state: &AppState, _team_id: &str, caller: &CallerContext) -> String {
    match caller {
        CallerContext::Lead => "lead".to_string(),
        CallerContext::Worker { profile_id } => {
            if let Ok(conn) = state.db.readers.get() {
                if let Ok(Some(m)) = db::member::get(&conn, profile_id) {
                    return m.name;
                }
            }
            profile_id.clone()
        }
    }
}
