use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub id: String,
    pub team_id: String,
    pub member_id: String,
    pub file_path: String,
    pub change_type: String,
    pub commit_hash: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: String,
    pub team_id: String,
    pub member_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub created_at: String,
}

pub fn insert_file_change(
    conn: &Connection,
    team_id: &str,
    member_id: &str,
    file_path: &str,
    change_type: &str,
    commit_hash: Option<&str>,
) -> Result<FileChange> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO file_changes (id, team_id, member_id, file_path, change_type, commit_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, team_id, member_id, file_path, change_type, commit_hash, now],
    )?;
    Ok(FileChange {
        id,
        team_id: team_id.to_string(),
        member_id: member_id.to_string(),
        file_path: file_path.to_string(),
        change_type: change_type.to_string(),
        commit_hash: commit_hash.map(str::to_string),
        created_at: now,
    })
}

/// Get member IDs who have touched the given file paths (for conflict resolution).
pub fn get_members_for_files(
    conn: &Connection,
    team_id: &str,
    file_paths: &[String],
) -> Result<Vec<String>> {
    if file_paths.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: String = file_paths
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT DISTINCT member_id FROM file_changes WHERE team_id = ?1 AND file_path IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&team_id];
    for fp in file_paths {
        params_vec.push(fp);
    }
    let members = stmt
        .query_map(params_vec.as_slice(), |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(members)
}

pub fn insert_agent_event(
    conn: &Connection,
    team_id: &str,
    member_id: &str,
    event_type: &str,
    payload: Option<&str>,
) -> Result<AgentEvent> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_events (id, team_id, member_id, event_type, payload, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, team_id, member_id, event_type, payload, now],
    )?;
    Ok(AgentEvent {
        id,
        team_id: team_id.to_string(),
        member_id: member_id.to_string(),
        event_type: event_type.to_string(),
        payload: payload.map(str::to_string),
        created_at: now,
    })
}

pub fn get_latest_event(
    conn: &Connection,
    member_id: &str,
    event_type: &str,
) -> Result<Option<AgentEvent>> {
    let result = conn
        .query_row(
            "SELECT id, team_id, member_id, event_type, payload, created_at
             FROM agent_events WHERE member_id = ?1 AND event_type = ?2
             ORDER BY created_at DESC LIMIT 1",
            params![member_id, event_type],
            |row| {
                Ok(AgentEvent {
                    id: row.get(0)?,
                    team_id: row.get(1)?,
                    member_id: row.get(2)?,
                    event_type: row.get(3)?,
                    payload: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )
        .ok();
    Ok(result)
}
