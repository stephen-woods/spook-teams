use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::db::{self, member::MemberStatus, message::MessageType};
use crate::server::{AppState, CallerContext, ReadInboxParams, SendMessageParams};
use crate::task::resolve_caller_member_id;

// ── Return types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct SendMessageResult {
    pub message_id: String,
    pub recipient: String,
    pub pushed: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadInboxResult {
    pub messages: Vec<MessageEntry>,
    pub count: usize,
    pub marked_read: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageEntry {
    pub id: String,
    pub sender_id: String,
    pub recipient: String,
    pub message_type: String,
    pub body: String,
    pub is_read: bool,
    pub created_at: String,
}

impl From<db::message::Message> for MessageEntry {
    fn from(m: db::message::Message) -> Self {
        MessageEntry {
            id: m.id,
            sender_id: m.sender_id,
            recipient: m.recipient,
            message_type: m.message_type.as_str().to_string(),
            body: m.body,
            is_read: m.is_read,
            created_at: m.created_at,
        }
    }
}

// ── send_message (8.1 + 8.3) ─────────────────────────────────────────────────

pub async fn send_message(
    state: &AppState,
    params: SendMessageParams,
    caller: CallerContext,
) -> Result<SendMessageResult> {
    let sender_id = resolve_caller_member_id(state, &params.team_id, &caller)?;

    // Store message
    let message = {
        let topic = if params.recipient.starts_with('#') {
            Some(params.recipient.as_str())
        } else {
            None
        };
        let conn = state.db.writer.lock().unwrap();
        db::message::insert(
            &conn,
            &params.team_id,
            &sender_id,
            &params.recipient,
            topic,
            MessageType::Text,
            &params.body,
            params.metadata.as_deref(),
        )?
    };

    // Route message via OpenCode SDK
    let (pushed, warning) = route_message(state, &params.team_id, &params.recipient, &params.body).await;

    Ok(SendMessageResult {
        message_id: message.id,
        recipient: params.recipient,
        pushed,
        warning,
    })
}

// ── read_inbox (8.2) ─────────────────────────────────────────────────────────

pub async fn read_inbox(
    state: &AppState,
    params: ReadInboxParams,
    caller: CallerContext,
) -> Result<ReadInboxResult> {
    let member_id = resolve_caller_member_id(state, &params.team_id, &caller)?;
    let unread_only = params.unread_only.unwrap_or(false);

    let messages = {
        let conn = state.db.readers.get()?;
        db::message::get_inbox(&conn, &params.team_id, &member_id, unread_only)?
    };

    let count = messages.len();

    // Mark messages as read
    let marked_read = if unread_only {
        let conn = state.db.writer.lock().unwrap();
        db::message::mark_read(&conn, &params.team_id, &member_id)?
    } else {
        0
    };

    Ok(ReadInboxResult {
        messages: messages.into_iter().map(MessageEntry::from).collect(),
        count,
        marked_read,
    })
}

// ── Routing logic (8.3) ──────────────────────────────────────────────────────

/// Route a message to recipient(s) via OpenCode SDK.
/// Returns (pushed, warning).
async fn route_message(
    state: &AppState,
    team_id: &str,
    recipient: &str,
    body: &str,
) -> (bool, Option<String>) {
    if recipient.starts_with('@') {
        // Direct message: @agent-name
        let agent_name = &recipient[1..];
        match push_direct(state, team_id, agent_name, body).await {
            Ok(true) => (true, None),
            Ok(false) => (
                false,
                Some(format!(
                    "Agent '{}' is not currently active. Message stored for catch-up.",
                    agent_name
                )),
            ),
            Err(e) => (false, Some(format!("Failed to push: {}", e))),
        }
    } else if recipient == "#team" {
        // Broadcast to all active agents
        match push_broadcast(state, team_id, body).await {
            Ok(n) => (n > 0, if n == 0 { Some("No active agents to notify.".to_string()) } else { None }),
            Err(e) => (false, Some(format!("Broadcast failed: {}", e))),
        }
    } else if recipient == "#conflict" {
        // Conflict topic — broadcast to active agents
        match push_broadcast(state, team_id, body).await {
            Ok(n) => (n > 0, if n == 0 { Some("No active agents to notify.".to_string()) } else { None }),
            Err(e) => (false, Some(format!("Conflict broadcast failed: {}", e))),
        }
    } else {
        // Unknown recipient — stored but not pushed
        (false, Some(format!("Unknown recipient format: '{}'. Use @agent-name, #team, or #conflict.", recipient)))
    }
}

/// Push directly to a named agent's OpenCode session.
async fn push_direct(
    state: &AppState,
    team_id: &str,
    agent_name: &str,
    body: &str,
) -> Result<bool> {
    let (member_id, session_id) = {
        let conn = state.db.readers.get()?;
        let member = db::member::get_by_name(&conn, team_id, agent_name)?;
        match member {
            None => return Ok(false),
            Some(m) => {
                if m.status == MemberStatus::Active {
                    let session_id = m.session_id.clone();
                    (m.id, session_id)
                } else {
                    return Ok(false);
                }
            }
        }
    };

    let clients = state.dispatcher.clients.read().await;
    if let Some(client) = clients.get(&member_id) {
        if let Some(sid) = session_id {
            let client = client.clone();
            let body = body.to_string();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&sid, &body).await {
                    warn!("Failed to push direct message: {}", e);
                }
            });
            return Ok(true);
        }
    }
    Ok(false)
}

/// Broadcast to all active agents.
async fn push_broadcast(state: &AppState, team_id: &str, body: &str) -> Result<usize> {
    let active = {
        let conn = state.db.readers.get()?;
        db::member::list_active_by_team(&conn, team_id)?
    };

    let clients = state.dispatcher.clients.read().await;
    let mut pushed = 0;
    for member in active {
        if let Some(client) = clients.get(&member.id) {
            if let Some(ref sid) = member.session_id {
                let client = client.clone();
                let body = body.to_string();
                let sid = sid.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.push_prompt(&sid, &body).await {
                        warn!("Failed to broadcast message: {}", e);
                    }
                });
                pushed += 1;
            }
        }
    }
    Ok(pushed)
}
