use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub change_name: String,
    pub project_path: String,
    pub tasks_path: String,
    pub status: TeamStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TeamStatus {
    Active,
    Completed,
    Paused,
}

impl TeamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TeamStatus::Active => "active",
            TeamStatus::Completed => "completed",
            TeamStatus::Paused => "paused",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "completed" => TeamStatus::Completed,
            "paused" => TeamStatus::Paused,
            _ => TeamStatus::Active,
        }
    }
}

fn from_row(row: &Row) -> rusqlite::Result<Team> {
    Ok(Team {
        id: row.get(0)?,
        name: row.get(1)?,
        change_name: row.get(2)?,
        project_path: row.get(3)?,
        tasks_path: row.get(4)?,
        status: TeamStatus::from_str(&row.get::<_, String>(5)?),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

const SELECT: &str =
    "SELECT id, name, change_name, project_path, tasks_path, status, created_at, updated_at FROM teams";

pub fn create(
    conn: &Connection,
    name: &str,
    change_name: &str,
    project_path: &str,
    tasks_path: &str,
) -> Result<Team> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO teams (id, name, change_name, project_path, tasks_path, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7)",
        params![id, name, change_name, project_path, tasks_path, now, now],
    )?;
    Ok(Team {
        id,
        name: name.to_string(),
        change_name: change_name.to_string(),
        project_path: project_path.to_string(),
        tasks_path: tasks_path.to_string(),
        status: TeamStatus::Active,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Team>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE id = ?1"))?;
    Ok(stmt.query_row(params![id], from_row).optional()?)
}

pub fn get_by_change_name(conn: &Connection, change_name: &str) -> Result<Option<Team>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE change_name = ?1 AND status = 'active'"
    ))?;
    Ok(stmt.query_row(params![change_name], from_row).optional()?)
}

pub fn update_status(conn: &Connection, id: &str, status: TeamStatus) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE teams SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<Team>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY created_at DESC"))?;
    let teams = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(teams)
}
