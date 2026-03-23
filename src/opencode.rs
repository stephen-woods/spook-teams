use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

// ── Port allocator ────────────────────────────────────────────────────────────

/// Tracks port assignment for opencode serve instances.
pub struct PortAllocator {
    base_port: u16,
    in_use: Mutex<std::collections::HashSet<u16>>,
}

impl PortAllocator {
    pub fn new(base_port: u16) -> Self {
        Self {
            base_port,
            in_use: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Allocate the next available port.
    pub fn allocate(&self) -> Result<u16> {
        let mut in_use = self.in_use.lock().unwrap();
        let mut port = self.base_port;
        loop {
            if !in_use.contains(&port) && !is_port_in_use(port) {
                in_use.insert(port);
                return Ok(port);
            }
            port = port.checked_add(1).context("No ports available")?;
        }
    }

    /// Release a port back to the pool.
    pub fn release(&self, port: u16) {
        self.in_use.lock().unwrap().remove(&port);
    }
}

fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

// ── OpenCode HTTP client ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CreateSessionRequest {
    agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PushPromptRequest {
    text: String,
}

/// HTTP client for a running `opencode serve` instance.
#[derive(Clone)]
pub struct OpenCodeClient {
    pub base_url: String,
    pub port: u16,
    http: reqwest::Client,
}

impl OpenCodeClient {
    pub fn new(port: u16) -> Self {
        Self {
            base_url: format!("http://127.0.0.1:{}", port),
            port,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Probe the health endpoint until it responds or timeout is reached.
    pub async fn wait_healthy(&self, timeout_secs: u64) -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            if std::time::Instant::now() > deadline {
                anyhow::bail!("Timeout waiting for opencode serve on port {} to become healthy", self.port);
            }
            if let Ok(resp) = self.http.get(&format!("{}/health", self.base_url)).send().await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
        }
    }

    /// Create a new OpenCode session.
    pub async fn create_session(&self, agent_type: &str) -> Result<Session> {
        let resp = self
            .http
            .post(&format!("{}/session", self.base_url))
            .json(&CreateSessionRequest {
                agent: agent_type.to_string(),
            })
            .send()
            .await
            .context("Failed to create session")?;
        let session: Session = resp.json().await.context("Failed to parse session response")?;
        Ok(session)
    }

    /// Push a prompt text to a session.
    pub async fn push_prompt(&self, session_id: &str, text: &str) -> Result<()> {
        self.http
            .post(&format!("{}/session/{}/prompt", self.base_url, session_id))
            .json(&PushPromptRequest {
                text: text.to_string(),
            })
            .send()
            .await
            .context("Failed to push prompt")?;
        Ok(())
    }
}

// ── Process management ────────────────────────────────────────────────────────

/// A managed opencode serve child process.
pub struct ManagedProcess {
    pub port: u16,
    pub worktree: std::path::PathBuf,
    child: Child,
}

/// Spawn `opencode serve` as a child process in the given working directory.
pub async fn spawn_serve(port: u16, worktree: &std::path::Path) -> Result<ManagedProcess> {
    let child = Command::new("opencode")
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(worktree)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn opencode serve")?;

    tracing::info!(port, worktree = %worktree.display(), "Spawned opencode serve");
    Ok(ManagedProcess {
        port,
        worktree: worktree.to_path_buf(),
        child,
    })
}

impl ManagedProcess {
    /// Wait for the process to exit and return the exit status.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        let status = self.child.wait().await.context("Failed to wait for child process")?;
        Ok(status)
    }

    /// Kill the process (SIGTERM, then SIGKILL).
    pub async fn kill(&mut self) -> Result<()> {
        // Send SIGTERM
        if let Err(e) = self.child.start_kill() {
            tracing::warn!("Failed to kill child process: {}", e);
        }
        // Wait briefly, then force kill
        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!("Process did not exit after SIGTERM, sending SIGKILL");
                let _ = self.child.kill().await;
            }
        }
        Ok(())
    }

    /// Get the process ID.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

// ── SSE event stream ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
}

/// Subscribe to an agent's SSE event stream.
/// Events are sent on the returned `broadcast::Receiver`.
/// The subscription task runs in the background and reconnects on failure.
pub async fn subscribe_sse(
    base_url: String,
    agent_id: String,
    tx: broadcast::Sender<SseEvent>,
) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("{}/event", base_url);
        let mut retry_delay = Duration::from_millis(500);
        let max_delay = Duration::from_secs(30);

        loop {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(agent_id, "SSE stream connected");
                    retry_delay = Duration::from_millis(500); // reset on success

                    use futures_util::StreamExt;
                    let mut stream = resp.bytes_stream();

                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(bytes) => {
                                // Parse SSE format: "event: TYPE\ndata: JSON\n\n"
                                let text = String::from_utf8_lossy(&bytes);
                                let event = parse_sse_event(&text);
                                if let Some(evt) = event {
                                    let _ = tx.send(SseEvent {
                                        event_type: evt.0,
                                        data: evt.1,
                                    });
                                }
                            }
                            Err(e) => {
                                tracing::warn!(agent_id, "SSE stream error: {}", e);
                                break;
                            }
                        }
                    }
                }
                Ok(resp) => {
                    tracing::warn!(agent_id, status = %resp.status(), "SSE endpoint returned non-200");
                }
                Err(e) => {
                    tracing::warn!(agent_id, "SSE connection failed: {}", e);
                }
            }

            // Reconnect with backoff
            tracing::debug!(agent_id, delay_ms = retry_delay.as_millis(), "SSE reconnecting");
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(max_delay);
        }
    });
}

fn parse_sse_event(text: &str) -> Option<(String, String)> {
    let mut event_type = String::new();
    let mut data = String::new();
    for line in text.lines() {
        if let Some(t) = line.strip_prefix("event: ") {
            event_type = t.to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data = d.to_string();
        }
    }
    if !event_type.is_empty() {
        Some((event_type, data))
    } else if !data.is_empty() {
        Some(("message".to_string(), data))
    } else {
        None
    }
}
