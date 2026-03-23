use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub team_id: String,
    pub sender_id: String,
    pub recipient: String,
    pub topic: Option<String>,
    pub message_type: MessageType,
    pub body: String,
    pub metadata: Option<String>,
    pub is_read: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    TaskComplete,
    TaskFail,
    MergeSuccess,
    MergeConflict,
    ConflictNegotiation,
    Crash,
    Convergence,
}

impl MessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessageType::Text => "text",
            MessageType::TaskComplete => "task_complete",
            MessageType::TaskFail => "task_fail",
            MessageType::MergeSuccess => "merge_success",
            MessageType::MergeConflict => "merge_conflict",
            MessageType::ConflictNegotiation => "conflict_negotiation",
            MessageType::Crash => "crash",
            MessageType::Convergence => "convergence",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "task_complete" => MessageType::TaskComplete,
            "task_fail" => MessageType::TaskFail,
            "merge_success" => MessageType::MergeSuccess,
            "merge_conflict" => MessageType::MergeConflict,
            "conflict_negotiation" => MessageType::ConflictNegotiation,
            "crash" => MessageType::Crash,
            "convergence" => MessageType::Convergence,
            _ => MessageType::Text,
        }
    }
}

fn from_row(row: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        team_id: row.get(1)?,
        sender_id: row.get(2)?,
        recipient: row.get(3)?,
        topic: row.get(4)?,
        message_type: MessageType::from_str(&row.get::<_, String>(5)?),
        body: row.get(6)?,
        metadata: row.get(7)?,
        is_read: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
    })
}

const SELECT: &str =
    "SELECT id, team_id, sender_id, recipient, topic, message_type, body, metadata, is_read, created_at FROM messages";

pub fn insert(
    conn: &Connection,
    team_id: &str,
    sender_id: &str,
    recipient: &str,
    topic: Option<&str>,
    message_type: MessageType,
    body: &str,
    metadata: Option<&str>,
) -> Result<Message> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO messages (id, team_id, sender_id, recipient, topic, message_type, body, metadata, is_read, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
        params![
            id,
            team_id,
            sender_id,
            recipient,
            topic,
            message_type.as_str(),
            body,
            metadata,
            now
        ],
    )?;
    Ok(Message {
        id,
        team_id: team_id.to_string(),
        sender_id: sender_id.to_string(),
        recipient: recipient.to_string(),
        topic: topic.map(str::to_string),
        message_type,
        body: body.to_string(),
        metadata: metadata.map(str::to_string),
        is_read: false,
        created_at: now,
    })
}

/// Get messages for a specific recipient or topic, optionally only unread.
pub fn get_inbox(
    conn: &Connection,
    team_id: &str,
    recipient: &str,
    unread_only: bool,
) -> Result<Vec<Message>> {
    let read_filter = if unread_only { " AND is_read = 0" } else { "" };
    let sql = format!(
        "{SELECT} WHERE team_id = ?1 AND (recipient = ?2 OR recipient = '#team'){read_filter}
         ORDER BY created_at"
    );
    let mut stmt = conn.prepare(&sql)?;
    let messages = stmt
        .query_map(params![team_id, recipient], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(messages)
}

/// Mark messages as read for a recipient.
pub fn mark_read(conn: &Connection, team_id: &str, recipient: &str) -> Result<usize> {
    let count = conn.execute(
        "UPDATE messages SET is_read = 1 WHERE team_id = ?1 AND recipient = ?2 AND is_read = 0",
        params![team_id, recipient],
    )?;
    Ok(count)
}
