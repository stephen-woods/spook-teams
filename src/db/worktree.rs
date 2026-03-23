use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub id: String,
    pub team_id: String,
    pub member_id: String,
    pub path: String,
    pub branch: String,
    pub base_commit: Option<String>,
    pub status: WorktreeStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeStatus {
    Active,
    Merged,
    CleanedUp,
}

impl WorktreeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorktreeStatus::Active => "active",
            WorktreeStatus::Merged => "merged",
            WorktreeStatus::CleanedUp => "cleaned_up",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "merged" => WorktreeStatus::Merged,
            "cleaned_up" => WorktreeStatus::CleanedUp,
            _ => WorktreeStatus::Active,
        }
    }
}

fn from_row(row: &Row) -> rusqlite::Result<Worktree> {
    Ok(Worktree {
        id: row.get(0)?,
        team_id: row.get(1)?,
        member_id: row.get(2)?,
        path: row.get(3)?,
        branch: row.get(4)?,
        base_commit: row.get(5)?,
        status: WorktreeStatus::from_str(&row.get::<_, String>(6)?),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

const SELECT: &str =
    "SELECT id, team_id, member_id, path, branch, base_commit, status, created_at, updated_at FROM worktrees";

pub fn create(
    conn: &Connection,
    team_id: &str,
    member_id: &str,
    path: &str,
    branch: &str,
    base_commit: Option<&str>,
) -> Result<Worktree> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO worktrees (id, team_id, member_id, path, branch, base_commit, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8)",
        params![id, team_id, member_id, path, branch, base_commit, now, now],
    )?;
    Ok(Worktree {
        id,
        team_id: team_id.to_string(),
        member_id: member_id.to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        base_commit: base_commit.map(str::to_string),
        status: WorktreeStatus::Active,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn get_by_member(conn: &Connection, member_id: &str) -> Result<Option<Worktree>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE member_id = ?1"))?;
    let result = stmt
        .query_map(params![member_id], from_row)?
        .next()
        .transpose()?;
    Ok(result)
}

pub fn update_status(conn: &Connection, id: &str, status: WorktreeStatus) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE worktrees SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    Ok(())
}

pub fn update_base_commit(conn: &Connection, id: &str, base_commit: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE worktrees SET base_commit = ?1, updated_at = ?2 WHERE id = ?3",
        params![base_commit, now, id],
    )?;
    Ok(())
}

pub fn list_by_team(conn: &Connection, team_id: &str) -> Result<Vec<Worktree>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE team_id = ?1 ORDER BY created_at"))?;
    let wts = stmt
        .query_map(params![team_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(wts)
}
