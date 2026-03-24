use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::db::{self, member::MemberRole, member::MemberStatus, task::TaskFilter};
use crate::server::{AppState, TeamCreateParams, TeamEndParams, TeamStatusParams};

// ── Return types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct TeamCreateResult {
    pub team_id: String,
    pub task_count: usize,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TeamStatusResult {
    pub team_id: String,
    pub team_name: String,
    pub status: String,
    pub change_name: String,
    pub members: Vec<MemberSummary>,
    pub task_counts: db::task::TaskCounts,
    pub progress_pct: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemberSummary {
    pub id: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub current_task: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TeamEndResult {
    pub team_id: String,
    pub tasks_exported: bool,
    pub agents_killed: usize,
    pub worktrees_cleaned: usize,
    pub message: String,
}

// ── team_create (5.1) ─────────────────────────────────────────────────────────

pub async fn team_create(state: &AppState, params: TeamCreateParams) -> Result<TeamCreateResult> {
    let project_path = params
        .project_path
        .unwrap_or_else(|| state.config.project_path.to_string_lossy().to_string());

    // Locate the tasks.md for the change
    let tasks_path = std::path::PathBuf::from(&project_path)
        .join("openspec")
        .join("changes")
        .join(&params.change_name)
        .join("tasks.md");

    if !tasks_path.exists() {
        anyhow::bail!(
            "tasks.md not found at {}. Is the change name correct?",
            tasks_path.display()
        );
    }

    // Check for existing active team for this change
    {
        let conn = state.db.readers.get()?;
        if db::team::get_by_change_name(&conn, &params.change_name)?.is_some() {
            anyhow::bail!(
                "An active team already exists for change '{}'. Use team_status or team_end first.",
                params.change_name
            );
        }
    }

    // Create the team record
    let tasks_path_str = tasks_path.to_string_lossy().to_string();
    let team = {
        let conn = state.db.writer.lock().unwrap();
        db::team::create(
            &conn,
            &params.name,
            &params.change_name,
            &project_path,
            &tasks_path_str,
        )?
    };

    // Import tasks from tasks.md
    let task_count = crate::bridge::import_tasks(&state.db, &team.id, &tasks_path)?;

    // Register lead as a member
    {
        let conn = state.db.writer.lock().unwrap();
        db::member::create(&conn, &team.id, "lead", MemberRole::Lead)?;
    }

    info!(
        team_id = %team.id,
        change_name = %params.change_name,
        task_count,
        "Team created"
    );

    Ok(TeamCreateResult {
        team_id: team.id,
        task_count,
        message: format!(
            "Team '{}' created for change '{}'. Imported {} tasks. Use agent_profile_create to set up workers.",
            params.name, params.change_name, task_count
        ),
    })
}

// ── team_status (5.2) ─────────────────────────────────────────────────────────

pub async fn team_status(state: &AppState, params: TeamStatusParams) -> Result<TeamStatusResult> {
    let team_id = match params.team_id {
        Some(id) => id,
        None => {
            // Find the most recently active team
            let conn = state.db.readers.get()?;
            let teams = db::team::list(&conn)?;
            teams
                .into_iter()
                .find(|t| t.status == db::team::TeamStatus::Active)
                .ok_or_else(|| anyhow::anyhow!("No active team found. Use team_id parameter."))?
                .id
        }
    };

    let (team, members, task_counts) = {
        let conn = state.db.readers.get()?;
        let team = db::team::get(&conn, &team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found: {}", team_id))?;
        let members = db::member::list_by_team(&conn, &team_id)?;
        let task_counts = db::task::count_by_status(&conn, &team_id)?;
        (team, members, task_counts)
    };

    // Build member summaries with current task
    let mut member_summaries = Vec::new();
    for member in members {
        let current_task = {
            let conn = state.db.readers.get()?;
            db::task::list(&conn, &team_id, TaskFilter::Mine(member.id.clone()))?
                .into_iter()
                .find(|t| t.status == db::task::TaskStatus::InProgress)
                .map(|t| t.title)
        };
        member_summaries.push(MemberSummary {
            id: member.id,
            name: member.name,
            role: member.role.as_str().to_string(),
            status: member.status.as_str().to_string(),
            current_task,
        });
    }

    let progress_pct = task_counts.progress_pct();

    Ok(TeamStatusResult {
        team_id: team.id,
        team_name: team.name,
        status: team.status.as_str().to_string(),
        change_name: team.change_name,
        members: member_summaries,
        task_counts,
        progress_pct,
    })
}

// ── team_end (5.3) ────────────────────────────────────────────────────────────

pub async fn team_end(state: &AppState, params: TeamEndParams) -> Result<TeamEndResult> {
    let cleanup_worktrees = params.cleanup_worktrees.unwrap_or(true);

    // Verify team exists
    let team = {
        let conn = state.db.readers.get()?;
        db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found: {}", params.team_id))?
    };

    // Export task states back to tasks.md
    let tasks_path = std::path::PathBuf::from(&team.tasks_path);
    let tasks_exported = if tasks_path.exists() {
        match crate::bridge::export_tasks(&state.db, &params.team_id, &tasks_path) {
            Ok(_) => {
                info!(team_id = %params.team_id, "Exported task states to tasks.md");
                true
            }
            Err(e) => {
                warn!("Failed to export tasks: {}", e);
                false
            }
        }
    } else {
        false
    };

    // Kill active agents
    let active_members = {
        let conn = state.db.readers.get()?;
        db::member::list_active_by_team(&conn, &params.team_id)?
    };

    let mut agents_killed = 0;
    let processes = state.processes.write().await;
    for member in &active_members {
        if let Some(proc) = processes.get(&member.id) {
            let mut proc = proc.lock().await;
            if let Err(e) = proc.kill().await {
                warn!(member_id = %member.id, "Failed to kill agent: {}", e);
            } else {
                agents_killed += 1;
            }
        }
        // Mark as completed
        let conn = state.db.writer.lock().unwrap();
        let _ = db::member::update_status(&conn, &member.id, MemberStatus::Completed);
    }
    drop(processes);

    // Clean up worktrees
    let mut worktrees_cleaned = 0;
    if cleanup_worktrees {
        let worktrees = {
            let conn = state.db.readers.get()?;
            db::worktree::list_by_team(&conn, &params.team_id)?
        };
        for wt in &worktrees {
            let wt_path = std::path::Path::new(&wt.path);
            if wt_path.exists() {
                // Remove worktree via git CLI
                let result = tokio::process::Command::new("git")
                    .args(["worktree", "remove", "--force", &wt.path])
                    .current_dir(&team.project_path)
                    .output()
                    .await;

                match result {
                    Ok(out) if out.status.success() => {
                        worktrees_cleaned += 1;
                        let conn = state.db.writer.lock().unwrap();
                        let _ = db::worktree::update_status(
                            &conn,
                            &wt.id,
                            db::worktree::WorktreeStatus::CleanedUp,
                        );
                    }
                    Ok(out) => {
                        warn!(
                            "git worktree remove failed for {}: {}",
                            wt.path,
                            String::from_utf8_lossy(&out.stderr)
                        );
                    }
                    Err(e) => {
                        warn!("Failed to run git worktree remove: {}", e);
                    }
                }
            }
        }
    }

    // Mark team as completed
    {
        let conn = state.db.writer.lock().unwrap();
        db::team::update_status(&conn, &params.team_id, db::team::TeamStatus::Completed)?;
    }

    // Cancel HTTP server
    state.http_cancel.cancel();

    Ok(TeamEndResult {
        team_id: params.team_id,
        tasks_exported,
        agents_killed,
        worktrees_cleaned,
        message: format!(
            "Team ended. {} agents killed, {} worktrees cleaned up.{}",
            agents_killed,
            worktrees_cleaned,
            if tasks_exported {
                " Task states exported to tasks.md."
            } else {
                ""
            }
        ),
    })
}
