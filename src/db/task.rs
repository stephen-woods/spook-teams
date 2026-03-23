use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub team_id: String,
    pub source_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub owner_id: Option<String>,
    pub section: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Blocked,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Blocked => "blocked",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "blocked" => TaskStatus::Blocked,
            "in_progress" => TaskStatus::InProgress,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::Pending,
        }
    }
}

fn from_row(row: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        team_id: row.get(1)?,
        source_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: TaskStatus::from_str(&row.get::<_, String>(5)?),
        owner_id: row.get(6)?,
        section: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const SELECT: &str =
    "SELECT id, team_id, source_id, title, description, status, owner_id, section, created_at, updated_at FROM tasks";

pub fn create(
    conn: &Connection,
    team_id: &str,
    source_id: &str,
    title: &str,
    description: Option<&str>,
    section: Option<&str>,
    status: TaskStatus,
) -> Result<Task> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO tasks (id, team_id, source_id, title, description, status, section, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            team_id,
            source_id,
            title,
            description,
            status.as_str(),
            section,
            now,
            now
        ],
    )?;
    Ok(Task {
        id,
        team_id: team_id.to_string(),
        source_id: source_id.to_string(),
        title: title.to_string(),
        description: description.map(str::to_string),
        status,
        owner_id: None,
        section: section.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Task>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE id = ?1"))?;
    Ok(stmt.query_row(params![id], from_row).optional()?)
}

pub fn get_by_source_id(conn: &Connection, team_id: &str, source_id: &str) -> Result<Option<Task>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE team_id = ?1 AND source_id = ?2"))?;
    Ok(stmt
        .query_row(params![team_id, source_id], from_row)
        .optional()?)
}

/// Update task status.
pub fn update_status(conn: &Connection, id: &str, status: TaskStatus) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, id],
    )?;
    Ok(())
}

/// Update task owner and set status to in_progress.
pub fn claim(conn: &Connection, id: &str, owner_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET owner_id = ?1, status = 'in_progress', updated_at = ?2 WHERE id = ?3",
        params![owner_id, now, id],
    )?;
    Ok(())
}

/// List tasks for a team with an optional filter.
pub fn list(conn: &Connection, team_id: &str, filter: TaskFilter) -> Result<Vec<Task>> {
    let sql = match filter {
        TaskFilter::All => format!("{SELECT} WHERE team_id = ?1 ORDER BY source_id"),
        TaskFilter::Mine(ref owner_id) => format!(
            "{SELECT} WHERE team_id = ?1 AND owner_id = '{owner_id}' ORDER BY source_id"
        ),
        TaskFilter::Available => format!(
            "{SELECT} WHERE team_id = ?1 AND status = 'pending' AND owner_id IS NULL ORDER BY source_id"
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let tasks = stmt
        .query_map(params![team_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tasks)
}

pub enum TaskFilter {
    All,
    Mine(String),
    Available,
}

/// Count tasks by status for a team.
pub fn count_by_status(conn: &Connection, team_id: &str) -> Result<TaskCounts> {
    let mut stmt =
        conn.prepare("SELECT status, COUNT(*) FROM tasks WHERE team_id = ?1 GROUP BY status")?;
    let mut counts = TaskCounts::default();
    let rows = stmt.query_map(params![team_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, count) = row?;
        match status.as_str() {
            "pending" => counts.pending = count as u32,
            "blocked" => counts.blocked = count as u32,
            "in_progress" => counts.in_progress = count as u32,
            "completed" => counts.completed = count as u32,
            "failed" => counts.failed = count as u32,
            "cancelled" => counts.cancelled = count as u32,
            _ => {}
        }
    }
    Ok(counts)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TaskCounts {
    pub pending: u32,
    pub blocked: u32,
    pub in_progress: u32,
    pub completed: u32,
    pub failed: u32,
    pub cancelled: u32,
}

impl TaskCounts {
    pub fn total(&self) -> u32 {
        self.pending
            + self.blocked
            + self.in_progress
            + self.completed
            + self.failed
            + self.cancelled
    }

    pub fn progress_pct(&self) -> f32 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.completed as f32 / total as f32
    }
}
