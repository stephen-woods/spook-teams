use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub role: MemberRole,
    pub status: MemberStatus,
    pub session_id: Option<String>,
    pub port: Option<u16>,
    pub worktree_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Lead,
    Worker,
}

impl MemberRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberRole::Lead => "lead",
            MemberRole::Worker => "worker",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "lead" => MemberRole::Lead,
            _ => MemberRole::Worker,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    PendingSpawn,
    Active,
    Idle,
    Crashed,
    Killed,
    Completed,
}

impl MemberStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberStatus::PendingSpawn => "pending_spawn",
            MemberStatus::Active => "active",
            MemberStatus::Idle => "idle",
            MemberStatus::Crashed => "crashed",
            MemberStatus::Killed => "killed",
            MemberStatus::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => MemberStatus::Active,
            "idle" => MemberStatus::Idle,
            "crashed" => MemberStatus::Crashed,
            "killed" => MemberStatus::Killed,
            "completed" => MemberStatus::Completed,
            _ => MemberStatus::PendingSpawn,
        }
    }
}

fn from_row(row: &Row) -> rusqlite::Result<Member> {
    Ok(Member {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        role: MemberRole::from_str(&row.get::<_, String>(3)?),
        status: MemberStatus::from_str(&row.get::<_, String>(4)?),
        session_id: row.get(5)?,
        port: row.get::<_, Option<i64>>(6)?.map(|p| p as u16),
        worktree_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const SELECT: &str =
    "SELECT id, team_id, name, role, status, session_id, port, worktree_id, created_at, updated_at FROM members";

pub fn create(conn: &Connection, team_id: &str, name: &str, role: MemberRole) -> Result<Member> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO members (id, team_id, name, role, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'pending_spawn', ?5, ?6)",
        params![id, team_id, name, role.as_str(), now, now],
    )?;
    Ok(Member {
        id,
        team_id: team_id.to_string(),
        name: name.to_string(),
        role,
        status: MemberStatus::PendingSpawn,
        session_id: None,
        port: None,
        worktree_id: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Member>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE id = ?1"))?;
    Ok(stmt.query_row(params![id], from_row).optional()?)
}

pub fn get_by_name(conn: &Connection, team_id: &str, name: &str) -> Result<Option<Member>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE team_id = ?1 AND name = ?2"))?;
    Ok(stmt
        .query_row(params![team_id, name], from_row)
        .optional()?)
}

pub fn update_status(conn: &Connection, id: &str, status: MemberStatus) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE members SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    Ok(())
}

pub fn update_session(conn: &Connection, id: &str, session_id: &str, port: u16) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE members SET session_id = ?1, port = ?2, updated_at = ?3 WHERE id = ?4",
        params![session_id, port as i64, now, id],
    )?;
    Ok(())
}

pub fn update_worktree(conn: &Connection, id: &str, worktree_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE members SET worktree_id = ?1, updated_at = ?2 WHERE id = ?3",
        params![worktree_id, now, id],
    )?;
    Ok(())
}

pub fn list_by_team(conn: &Connection, team_id: &str) -> Result<Vec<Member>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE team_id = ?1 ORDER BY created_at"))?;
    let members = stmt
        .query_map(params![team_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(members)
}

pub fn list_active_by_team(conn: &Connection, team_id: &str) -> Result<Vec<Member>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE team_id = ?1 AND status = 'active' ORDER BY created_at"
    ))?;
    let members = stmt
        .query_map(params![team_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(members)
}
