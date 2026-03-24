use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

use crate::db::{self, worktree::WorktreeStatus};
use crate::server::{
    AppState, CallerContext, GetAgentDiffParams, GetConflictDetailsParams, MergeToMainParams,
    RebaseFromMainParams, WorktreeCleanupParams, WorktreeStatusParams,
};
use crate::task::resolve_caller_member_id;

// ── Return types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct WorktreeStatusResult {
    pub agent_name: String,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub modified_files: Vec<String>,
    pub worktree_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MergeResult {
    pub success: bool,
    pub message: String,
    pub changed_files: Vec<String>,
    pub conflict_files: Vec<String>,
    pub counterpart_agents: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConflictDetails {
    pub conflict_files: Vec<ConflictFile>,
    pub counterpart_agents: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub ours: String,
    pub theirs: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentDiffResult {
    pub agent_name: String,
    pub diff: String,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RebaseResult {
    pub success: bool,
    pub message: String,
    pub conflict_files: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorktreeCleanupResult {
    pub member_id: String,
    pub path: String,
    pub branch_deleted: bool,
    pub message: String,
}

// ── worktree_create (6.1) — called from agent.rs ─────────────────────────────

/// Create a git worktree at `.worktrees/<name>/` with branch `teams/<name>`.
/// Returns (worktree_path, branch_name, base_commit).
pub async fn create_worktree_for_agent(
    project_path: &str,
    agent_name: &str,
) -> Result<(PathBuf, String, String)> {
    let project = PathBuf::from(project_path);
    let worktree_path = project.join(".worktrees").join(agent_name);
    let branch = format!("teams/{}", agent_name);

    // Get HEAD commit
    let head_out = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&project)
        .output()
        .await?;
    let base_commit = String::from_utf8_lossy(&head_out.stdout).trim().to_string();

    // Create worktrees directory if needed
    tokio::fs::create_dir_all(project.join(".worktrees")).await?;

    // Add worktree with new branch
    let out = tokio::process::Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            &branch,
            worktree_path.to_str().unwrap(),
            "HEAD",
        ])
        .current_dir(&project)
        .output()
        .await?;

    if !out.status.success() {
        anyhow::bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    info!(
        agent = agent_name,
        branch,
        path = %worktree_path.display(),
        "Created git worktree"
    );

    Ok((worktree_path, branch, base_commit))
}

// ── worktree_status (6.2) ─────────────────────────────────────────────────────

pub async fn worktree_status(
    state: &AppState,
    params: WorktreeStatusParams,
    caller: CallerContext,
) -> Result<WorktreeStatusResult> {
    // Determine which agent to query
    let agent_name = match params.agent_name {
        Some(name) => name,
        None => {
            let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
            let conn = state.db.readers.get()?;
            db::member::get(&conn, &member_id)?
                .map(|m| m.name)
                .unwrap_or_default()
        }
    };

    let wt = find_worktree_for_agent(state, &params.team_id, &agent_name)?;

    // Get ahead/behind counts
    let branch = format!("teams/{}", agent_name);
    let (ahead, behind) = get_divergence(&wt.path, &branch).await?;

    // Get modified files
    let modified_files = get_modified_files(&wt.path, &branch).await?;

    Ok(WorktreeStatusResult {
        agent_name,
        branch,
        ahead,
        behind,
        modified_files,
        worktree_path: wt.path,
    })
}

// ── merge_to_main (6.3) ───────────────────────────────────────────────────────

pub async fn merge_to_main(
    state: &AppState,
    params: MergeToMainParams,
    caller: CallerContext,
) -> Result<MergeResult> {
    let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
    let (agent_name, project_path) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found"))?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        (member.name, team.project_path)
    };

    let branch = format!("teams/{}", agent_name);
    let commit_msg = params.message.unwrap_or_else(|| {
        format!("Merge {} into main", branch)
    });

    // Attempt merge via git CLI from the project root
    let merge_out = tokio::process::Command::new("git")
        .args(["merge", "--no-ff", "-m", &commit_msg, &branch])
        .current_dir(&project_path)
        .output()
        .await?;

    if merge_out.status.success() {
        // Get changed files
        let changed_files = get_files_in_branch(&project_path, &branch).await?;

        // Record file changes
        for file in &changed_files {
            let conn = state.db.writer.lock().unwrap();
            let _ = db::file_changes::insert_file_change(
                &conn,
                &params.team_id,
                &member_id,
                file,
                "merge",
                None,
            );
        }

        // Update worktree status
        let wt = find_worktree_for_agent(state, &params.team_id, &agent_name)?;
        {
            let conn = state.db.writer.lock().unwrap();
            db::worktree::update_status(&conn, &wt.id, WorktreeStatus::Merged)?;
        }

        // Trigger merge success dispatcher
        state
            .dispatcher
            .on_merge_success(&params.team_id, &agent_name, &changed_files)
            .await?;

        Ok(MergeResult {
            success: true,
            message: format!("Branch '{}' merged to main successfully.", branch),
            changed_files,
            conflict_files: vec![],
            counterpart_agents: vec![],
        })
    } else {
        let stderr = String::from_utf8_lossy(&merge_out.stderr);

        // Abort the failed merge
        let _ = tokio::process::Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(&project_path)
            .output()
            .await;

        // Find conflicting files
        let conflict_files = parse_conflict_files(&stderr);

        // Find counterpart agents from file_changes
        let counterpart_agents = if !conflict_files.is_empty() {
            let conn = state.db.readers.get()?;
            let member_ids =
                db::file_changes::get_members_for_files(&conn, &params.team_id, &conflict_files)?;
            member_ids
                .into_iter()
                .filter(|id| id != &member_id)
                .filter_map(|id| {
                    let conn = state.db.readers.get().ok()?;
                    db::member::get(&conn, &id).ok()?.map(|m| m.name)
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        // Trigger conflict dispatcher
        if !counterpart_agents.is_empty() {
            if let Ok(Some(counterpart)) = {
                let conn = state.db.readers.get()?;
                db::member::get_by_name(&conn, &params.team_id, &counterpart_agents[0])
            } {
                state
                    .dispatcher
                    .on_merge_conflict(
                        &params.team_id,
                        &agent_name,
                        &counterpart.id,
                        &conflict_files,
                    )
                    .await?;
            }
        }

        Ok(MergeResult {
            success: false,
            message: format!(
                "Merge failed with conflicts. Merge aborted. Counterpart agents: {}",
                counterpart_agents.join(", ")
            ),
            changed_files: vec![],
            conflict_files,
            counterpart_agents,
        })
    }
}

// ── get_conflict_details (6.4) ───────────────────────────────────────────────

pub async fn get_conflict_details(
    state: &AppState,
    params: GetConflictDetailsParams,
    caller: CallerContext,
) -> Result<ConflictDetails> {
    let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
    let (_agent_name, project_path) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found"))?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        (member.name, team.project_path)
    };

    // Find conflicting files via git status
    let status_out = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&project_path)
        .output()
        .await?;

    let conflict_files: Vec<String> = String::from_utf8_lossy(&status_out.stdout)
        .lines()
        .filter(|l| l.starts_with("UU") || l.starts_with("AA") || l.starts_with("DD"))
        .map(|l| l[3..].trim().to_string())
        .collect();

    // Get conflict content for each file
    let mut conflict_file_details = Vec::new();
    for file in &conflict_files {
        let ours = get_conflict_side(&project_path, file, "--ours").await?;
        let theirs = get_conflict_side(&project_path, file, "--theirs").await?;
        conflict_file_details.push(ConflictFile {
            path: file.clone(),
            ours,
            theirs,
        });
    }

    // Find counterpart agents
    let counterpart_agents = {
        let conn = state.db.readers.get()?;
        let member_ids =
            db::file_changes::get_members_for_files(&conn, &params.team_id, &conflict_files)?;
        member_ids
            .into_iter()
            .filter(|id| id != &member_id)
            .filter_map(|id| {
                let conn = state.db.readers.get().ok()?;
                db::member::get(&conn, &id).ok()?.map(|m| m.name)
            })
            .collect::<Vec<_>>()
    };

    Ok(ConflictDetails {
        conflict_files: conflict_file_details,
        counterpart_agents,
    })
}

// ── get_agent_diff (6.5) ──────────────────────────────────────────────────────

pub async fn get_agent_diff(
    state: &AppState,
    params: GetAgentDiffParams,
) -> Result<AgentDiffResult> {
    let (project_path, branch) = {
        let conn = state.db.readers.get()?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        let branch = format!("teams/{}", params.agent_name);
        (team.project_path, branch)
    };

    // Find common ancestor with main
    let merge_base_out = tokio::process::Command::new("git")
        .args(["merge-base", "HEAD", &branch])
        .current_dir(&project_path)
        .output()
        .await?;
    let base = String::from_utf8_lossy(&merge_base_out.stdout).trim().to_string();

    // Get diff from common ancestor to branch tip
    let mut args = vec!["diff", &base, &branch];
    let file_arg;
    if let Some(ref file) = params.file_path {
        file_arg = file.clone();
        args.push("--");
        args.push(&file_arg);
    }

    let diff_out = tokio::process::Command::new("git")
        .args(&args)
        .current_dir(&project_path)
        .output()
        .await?;

    let diff = String::from_utf8_lossy(&diff_out.stdout).to_string();

    // Extract file names from diff
    let files: Vec<String> = diff
        .lines()
        .filter(|l| l.starts_with("+++ b/"))
        .map(|l| l[6..].to_string())
        .collect();

    Ok(AgentDiffResult {
        agent_name: params.agent_name,
        diff,
        files,
    })
}

// ── rebase_from_main (6.6) ────────────────────────────────────────────────────

pub async fn rebase_from_main(
    state: &AppState,
    params: RebaseFromMainParams,
    caller: CallerContext,
) -> Result<RebaseResult> {
    let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
    let (agent_name, _project_path) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get(&conn, &member_id)?
            .ok_or_else(|| anyhow::anyhow!("Member not found"))?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        (member.name, team.project_path)
    };

    let wt = find_worktree_for_agent(state, &params.team_id, &agent_name)?;
    let branch = format!("teams/{}", agent_name);

    // Rebase from the worktree directory
    let rebase_out = tokio::process::Command::new("git")
        .args(["rebase", "origin/main"])
        .current_dir(&wt.path)
        .output()
        .await?;

    if rebase_out.status.success() {
        // Update base commit
        let head_out = tokio::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&wt.path)
            .output()
            .await?;
        let new_base = String::from_utf8_lossy(&head_out.stdout).trim().to_string();

        {
            let conn = state.db.writer.lock().unwrap();
            db::worktree::update_base_commit(&conn, &wt.id, &new_base)?;
        }

        Ok(RebaseResult {
            success: true,
            message: format!("Branch '{}' rebased onto main successfully.", branch),
            conflict_files: vec![],
        })
    } else {
        let stderr = String::from_utf8_lossy(&rebase_out.stderr);
        let conflict_files = parse_conflict_files(&stderr);

        // Abort the rebase
        let _ = tokio::process::Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(&wt.path)
            .output()
            .await;

        Ok(RebaseResult {
            success: false,
            message: format!("Rebase failed with conflicts. Aborted. Conflicts: {}", conflict_files.join(", ")),
            conflict_files,
        })
    }
}

// ── worktree_cleanup (6.7) ────────────────────────────────────────────────────

pub async fn worktree_cleanup(
    state: &AppState,
    params: WorktreeCleanupParams,
) -> Result<WorktreeCleanupResult> {
    let (project_path, wt) = {
        let conn = state.db.readers.get()?;
        let team = db::team::get(&conn, &params.team_id)?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        let wt = db::worktree::get_by_member(&conn, &params.member_id)?
            .ok_or_else(|| anyhow::anyhow!("Worktree not found for member"))?;
        (team.project_path, wt)
    };

    let wt_path = wt.path.clone();
    let branch = wt.branch.clone();

    // Remove worktree
    let rm_out = tokio::process::Command::new("git")
        .args(["worktree", "remove", "--force", &wt_path])
        .current_dir(&project_path)
        .output()
        .await?;

    if !rm_out.status.success() {
        let stderr = String::from_utf8_lossy(&rm_out.stderr);
        anyhow::bail!("git worktree remove failed: {}", stderr);
    }

    // Optionally delete branch
    let delete_branch = params.delete_branch.unwrap_or(false);
    let branch_deleted = if delete_branch {
        let del_out = tokio::process::Command::new("git")
            .args(["branch", "-d", &branch])
            .current_dir(&project_path)
            .output()
            .await?;
        del_out.status.success()
    } else {
        false
    };

    // Update worktree status
    {
        let conn = state.db.writer.lock().unwrap();
        db::worktree::update_status(&conn, &wt.id, WorktreeStatus::CleanedUp)?;
    }

    info!(member_id = %params.member_id, path = %wt_path, "Worktree cleaned up");

    Ok(WorktreeCleanupResult {
        member_id: params.member_id,
        path: wt_path,
        branch_deleted,
        message: format!(
            "Worktree removed.{}",
            if branch_deleted {
                format!(" Branch '{}' deleted.", branch)
            } else {
                format!(" Branch '{}' preserved.", branch)
            }
        ),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn find_worktree_for_agent(
    state: &AppState,
    team_id: &str,
    agent_name: &str,
) -> Result<db::worktree::Worktree> {
    let conn = state.db.readers.get()?;
    let member = db::member::get_by_name(&conn, team_id, agent_name)?
        .ok_or_else(|| anyhow::anyhow!("Agent '{}' not found in team", agent_name))?;
    db::worktree::get_by_member(&conn, &member.id)?
        .ok_or_else(|| anyhow::anyhow!("No worktree found for agent '{}'", agent_name))
}

async fn get_divergence(worktree_path: &str, branch: &str) -> Result<(usize, usize)> {
    let out = tokio::process::Command::new("git")
        .args(["rev-list", "--left-right", "--count", &format!("HEAD...{}", branch)])
        .current_dir(worktree_path)
        .output()
        .await?;

    let s = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    let behind = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((ahead, behind))
}

async fn get_modified_files(worktree_path: &str, branch: &str) -> Result<Vec<String>> {
    let out = tokio::process::Command::new("git")
        .args(["diff", "--name-only", &format!("origin/main...{}", branch)])
        .current_dir(worktree_path)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

async fn get_files_in_branch(project_path: &str, branch: &str) -> Result<Vec<String>> {
    let out = tokio::process::Command::new("git")
        .args(["diff", "--name-only", &format!("origin/main...{}", branch)])
        .current_dir(project_path)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

async fn get_conflict_side(project_path: &str, file: &str, side: &str) -> Result<String> {
    let out = tokio::process::Command::new("git")
        .args(["show", &format!(":{}", side.trim_start_matches("--")), "--", file])
        .current_dir(project_path)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_conflict_files(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|l| l.contains("CONFLICT") || l.contains("conflict in"))
        .filter_map(|l| {
            // "CONFLICT (content): Merge conflict in path/to/file"
            if let Some(pos) = l.rfind(" in ") {
                Some(l[pos + 4..].trim().to_string())
            } else {
                None
            }
        })
        .collect()
}
