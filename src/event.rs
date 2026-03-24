use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::db;
use crate::db::Db;
use crate::opencode::OpenCodeClient;
use crate::spawner::{LogLevel, Spawner, StatusUpdate, WorkspaceId};

/// Event dispatcher — called synchronously from MCP tool handlers after state mutations.
/// Push notifications are spawned as fire-and-forget tasks.
pub struct EventDispatcher {
    pub db: Arc<Db>,
    pub spawner: Arc<dyn Spawner>,
    /// Active OpenCode clients per member_id
    pub clients: Arc<tokio::sync::RwLock<std::collections::HashMap<String, Arc<OpenCodeClient>>>>,
    /// Workspace IDs per member_id for cmux updates
    pub workspaces: Arc<tokio::sync::RwLock<std::collections::HashMap<String, WorkspaceId>>>,
}

impl EventDispatcher {
    pub fn new(
        db: Arc<Db>,
        spawner: Arc<dyn Spawner>,
    ) -> Self {
        Self {
            db,
            spawner,
            clients: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            workspaces: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    // ── Task completion dispatcher (10.1) ─────────────────────────────────────

    /// Called when a task is completed.
    /// - Unblocks dependent tasks
    /// - Pushes status to lead
    /// - Pushes availability to unblocked agents
    /// - Updates cmux progress
    pub async fn on_task_complete(
        &self,
        team_id: &str,
        task_id: &str,
        agent_name: &str,
    ) -> Result<()> {
        // Compute newly unblocked tasks
        let newly_unblocked = {
            let conn = self.db.readers.get()?;
            db::task_dep::compute_newly_unblocked(&conn, task_id)?
        };

        // Update newly unblocked tasks to pending
        for unblocked_id in &newly_unblocked {
            let conn = self.db.writer.lock().unwrap();
            db::task::update_status(&conn, unblocked_id, db::task::TaskStatus::Pending)?;
        }

        // Get task counts for progress
        let counts = {
            let conn = self.db.readers.get()?;
            db::task::count_by_status(&conn, team_id)?
        };

        let total = counts.total();
        let completed = counts.completed;
        let all_done = counts.pending == 0
            && counts.blocked == 0
            && counts.in_progress == 0
            && counts.failed == 0;

        // Get task title
        let task_title = {
            let conn = self.db.readers.get()?;
            db::task::get(&conn, task_id)?
                .map(|t| t.title)
                .unwrap_or_default()
        };

        // Find lead member
        let lead_id = self.find_lead_session(team_id).await;

        // Push completion status to lead
        if let Some((_lead_member_id, lead_session_id, client)) = lead_id {
            let msg = if all_done {
                format!(
                    "All tasks complete ({}/{}). Team is ready to converge and merge.",
                    completed, total
                )
            } else {
                format!(
                    "Agent {} completed task: {} ({}/{} done)",
                    agent_name, task_title, completed, total
                )
            };
            let client = client.clone();
            let session_id = lead_session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&session_id, &msg).await {
                    warn!("Failed to push task complete to lead: {}", e);
                }
            });

            // Trigger convergence dispatcher
            if all_done {
                self.on_convergence(team_id).await?;
            }

            // Update cmux progress for agent
            let _clients = self.clients.read().await;
            let workspaces = self.workspaces.read().await;
            // Find agent's member_id
            if let Ok(Some(member)) = {
                let conn = self.db.readers.get()?;
                db::member::get_by_name(&conn, team_id, agent_name)
            } {
                if let Some(ws) = workspaces.get(&member.id) {
                    let pct = completed as f32 / total as f32;
                    let label = format!("{}/{} tasks", completed, total);
                    let spawner = self.spawner.clone();
                    let ws = ws.clone();
                    tokio::spawn(async move {
                        let _ = spawner.set_progress(&ws, pct, &label).await;
                        let _ = spawner
                            .log(
                                &ws,
                                LogLevel::Success,
                                &format!("Completed: {}", task_title),
                            )
                            .await;
                    });
                }
            }
        }

        // Notify agents whose tasks got unblocked
        for unblocked_id in &newly_unblocked {
            if let Ok(Some(task)) = {
                let conn = self.db.readers.get()?;
                db::task::get(&conn, unblocked_id)
            } {
                if let Some(owner_id) = &task.owner_id {
                    if let Some((session_id, client)) = self.get_client_session(owner_id).await {
                        let msg = format!(
                            "Task '{}' is now unblocked and ready to work on.",
                            task.title
                        );
                        let client = client.clone();
                        tokio::spawn(async move {
                            if let Err(e) = client.push_prompt(&session_id, &msg).await {
                                warn!("Failed to notify agent of unblocked task: {}", e);
                            }
                        });
                    }
                }
            }
        }

        Ok(())
    }

    // ── Task failure dispatcher (10.2) ────────────────────────────────────────

    pub async fn on_task_fail(
        &self,
        team_id: &str,
        task_id: &str,
        agent_name: &str,
        reason: &str,
    ) -> Result<()> {
        let task_title = {
            let conn = self.db.readers.get()?;
            db::task::get(&conn, task_id)?
                .map(|t| t.title)
                .unwrap_or_default()
        };

        if let Some((_, lead_session_id, client)) = self.find_lead_session(team_id).await {
            let msg = format!(
                "Agent {} failed task '{}': {}",
                agent_name, task_title, reason
            );
            let client = client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&lead_session_id, &msg).await {
                    warn!("Failed to push task fail to lead: {}", e);
                }
            });
        }

        // Update cmux log
        if let Ok(Some(member)) = {
            let conn = self.db.readers.get()?;
            db::member::get_by_name(&conn, team_id, agent_name)
        } {
            let workspaces = self.workspaces.read().await;
            if let Some(ws) = workspaces.get(&member.id) {
                let spawner = self.spawner.clone();
                let ws = ws.clone();
                let msg = format!("Failed: {} — {}", task_title, reason);
                tokio::spawn(async move {
                    let _ = spawner.log(&ws, LogLevel::Error, &msg).await;
                });
            }
        }

        Ok(())
    }

    // ── Merge conflict dispatcher (10.3) ─────────────────────────────────────

    pub async fn on_merge_conflict(
        &self,
        team_id: &str,
        agent_name: &str,
        counterpart_member_id: &str,
        conflicting_files: &[String],
    ) -> Result<()> {
        let files_str = conflicting_files.join(", ");

        // Push to counterpart
        if let Some((session_id, client)) = self.get_client_session(counterpart_member_id).await {
            let msg = format!(
                "Merge conflict notification: Agent {} has conflicts with your changes on: {}. \
                 Please use get_conflict_details and get_agent_diff to negotiate resolution.",
                agent_name, files_str
            );
            let client = client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&session_id, &msg).await {
                    warn!("Failed to push conflict to counterpart: {}", e);
                }
            });
        }

        // Push to lead
        if let Some((_, lead_session_id, client)) = self.find_lead_session(team_id).await {
            let counterpart_name = {
                let conn = self.db.readers.get()?;
                db::member::get(&conn, counterpart_member_id)?
                    .map(|m| m.name)
                    .unwrap_or_else(|| counterpart_member_id.to_string())
            };
            let msg = format!(
                "Conflict detected: {} vs {} on {}. Agents are negotiating.",
                agent_name, counterpart_name, files_str
            );
            let client = client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&lead_session_id, &msg).await {
                    warn!("Failed to push conflict to lead: {}", e);
                }
            });
        }

        // Update cmux for agent
        if let Ok(Some(member)) = {
            let conn = self.db.readers.get()?;
            db::member::get_by_name(&conn, team_id, agent_name)
        } {
            let workspaces = self.workspaces.read().await;
            if let Some(ws) = workspaces.get(&member.id) {
                let spawner = self.spawner.clone();
                let ws = ws.clone();
                let msg = format!("Conflict on: {}", files_str);
                tokio::spawn(async move {
                    let _ = spawner
                        .set_status(
                            &ws,
                            &StatusUpdate {
                                text: "Conflict".to_string(),
                                icon: Some("⚠️".to_string()),
                                color: Some("yellow".to_string()),
                            },
                        )
                        .await;
                    let _ = spawner.log(&ws, LogLevel::Warning, &msg).await;
                    let _ = spawner
                        .notify("Merge Conflict", &format!("Conflict between agents on: {}", msg))
                        .await;
                });
            }
        }

        Ok(())
    }

    // ── Merge success dispatcher (10.4) ──────────────────────────────────────

    pub async fn on_merge_success(
        &self,
        team_id: &str,
        agent_name: &str,
        changed_files: &[String],
    ) -> Result<()> {
        let files_str = changed_files.join(", ");
        let msg = format!(
            "Agent {} successfully merged to main. Changed files: {}",
            agent_name, files_str
        );

        // Broadcast to all active agents
        let active_members = {
            let conn = self.db.readers.get()?;
            db::member::list_active_by_team(&conn, team_id)?
        };

        let clients = self.clients.read().await;
        for member in &active_members {
            if member.name == agent_name {
                continue;
            }
            if let Some(client) = clients.get(&member.id) {
                if let Some(ref session_id) = member.session_id {
                    let client = client.clone();
                    let session_id = session_id.clone();
                    let msg = msg.clone();
                    tokio::spawn(async move {
                        if let Err(e) = client.push_prompt(&session_id, &msg).await {
                            warn!("Failed to broadcast merge success: {}", e);
                        }
                    });
                }
            }
        }

        // Update cmux
        if let Ok(Some(member)) = {
            let conn = self.db.readers.get()?;
            db::member::get_by_name(&conn, team_id, agent_name)
        } {
            let workspaces = self.workspaces.read().await;
            if let Some(ws) = workspaces.get(&member.id) {
                let spawner = self.spawner.clone();
                let ws = ws.clone();
                let log_msg = format!("Merged: {}", files_str);
                tokio::spawn(async move {
                    let _ = spawner.log(&ws, LogLevel::Success, &log_msg).await;
                    let _ = spawner
                        .set_status(
                            &ws,
                            &StatusUpdate {
                                text: "Merged".to_string(),
                                icon: Some("✓".to_string()),
                                color: Some("green".to_string()),
                            },
                        )
                        .await;
                });
            }
        }

        Ok(())
    }

    // ── Crash dispatcher (10.5) ───────────────────────────────────────────────

    pub async fn on_crash(
        &self,
        team_id: &str,
        member_id: &str,
        last_task_id: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        // Mark agent as crashed
        {
            let conn = self.db.writer.lock().unwrap();
            db::member::update_status(&conn, member_id, db::member::MemberStatus::Crashed)?;
        }

        // Mark in-progress tasks as failed
        let in_progress_tasks = {
            let conn = self.db.readers.get()?;
            db::task::list(
                &conn,
                team_id,
                db::task::TaskFilter::Mine(member_id.to_string()),
            )?
            .into_iter()
            .filter(|t| t.status == db::task::TaskStatus::InProgress)
            .collect::<Vec<_>>()
        };

        for task in &in_progress_tasks {
            let conn = self.db.writer.lock().unwrap();
            db::task::update_status(&conn, &task.id, db::task::TaskStatus::Failed)?;
        }

        // Capture cmux screen if available
        let screen_content = {
            let workspaces = self.workspaces.read().await;
            if let Some(ws) = workspaces.get(member_id) {
                self.spawner.read_screen(ws).await.unwrap_or_default()
            } else {
                String::new()
            }
        };

        // Record crash event
        {
            let payload = serde_json::json!({
                "reason": reason,
                "last_task_id": last_task_id,
                "screen": screen_content,
                "failed_tasks": in_progress_tasks.iter().map(|t| &t.id).collect::<Vec<_>>()
            })
            .to_string();
            let conn = self.db.writer.lock().unwrap();
            db::file_changes::insert_agent_event(
                &conn,
                team_id,
                member_id,
                "crash",
                Some(&payload),
            )?;
        }

        // Get member info
        let member_name = {
            let conn = self.db.readers.get()?;
            db::member::get(&conn, member_id)?
                .map(|m| m.name)
                .unwrap_or_else(|| member_id.to_string())
        };

        // Push crash report to lead
        if let Some((_, lead_session_id, client)) = self.find_lead_session(team_id).await {
            let failed_task_titles: Vec<String> = in_progress_tasks
                .iter()
                .map(|t| t.title.clone())
                .collect();
            let msg = format!(
                "Agent {} crashed: {}. Failed tasks: {}. Screen output captured.",
                member_name,
                reason,
                failed_task_titles.join(", ")
            );
            let client = client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&lead_session_id, &msg).await {
                    warn!("Failed to push crash report to lead: {}", e);
                }
            });
        }

        // Update cmux
        let workspaces = self.workspaces.read().await;
        if let Some(ws) = workspaces.get(member_id) {
            let spawner = self.spawner.clone();
            let ws = ws.clone();
            let member_name_cl = member_name.clone();
            tokio::spawn(async move {
                let _ = spawner
                    .set_status(
                        &ws,
                        &StatusUpdate {
                            text: "Crashed".to_string(),
                            icon: Some("✗".to_string()),
                            color: Some("red".to_string()),
                        },
                    )
                    .await;
                let _ = spawner
                    .notify(
                        "Agent Crashed",
                        &format!("Agent {} crashed", member_name_cl),
                    )
                    .await;
            });
        }

        Ok(())
    }

    // ── Convergence dispatcher (10.6) ────────────────────────────────────────

    async fn on_convergence(&self, team_id: &str) -> Result<()> {
        if let Some((_, lead_session_id, client)) = self.find_lead_session(team_id).await {
            let msg = "All tasks complete. Ready to merge and converge. Run `team_end` to finalize.".to_string();
            let client = client.clone();
            tokio::spawn(async move {
                if let Err(e) = client.push_prompt(&lead_session_id, &msg).await {
                    warn!("Failed to push convergence to lead: {}", e);
                }
            });
        }
        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Find the lead member's session ID and client for the given team.
    async fn find_lead_session(
        &self,
        team_id: &str,
    ) -> Option<(String, String, Arc<OpenCodeClient>)> {
        let conn = self.db.readers.get().ok()?;
        let members = db::member::list_by_team(&conn, team_id).ok()?;
        let lead = members
            .into_iter()
            .find(|m| m.role == db::member::MemberRole::Lead)?;
        let session_id = lead.session_id.clone()?;
        let clients = self.clients.read().await;
        let client = clients.get(&lead.id)?.clone();
        Some((lead.id, session_id, client))
    }

    /// Get (session_id, client) for a given member_id.
    async fn get_client_session(&self, member_id: &str) -> Option<(String, Arc<OpenCodeClient>)> {
        let conn = self.db.readers.get().ok()?;
        let member = db::member::get(&conn, member_id).ok()??;
        let session_id = member.session_id?;
        let clients = self.clients.read().await;
        let client = clients.get(member_id)?.clone();
        Some((session_id, client))
    }
}
