use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::{LogLevel, SplitDirection, Spawner, StatusUpdate, SurfaceId, WorkspaceId};

/// JSON-RPC request to cmux.
#[derive(Serialize)]
struct RpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: Value,
}

/// JSON-RPC response from cmux.
#[derive(Deserialize)]
struct RpcResponse {
    #[allow(dead_code)]
    id: Option<u64>,
    result: Option<Value>,
    error: Option<Value>,
}

/// Known default cmux socket paths to try if `CMUX_SOCKET_PATH` is not set.
const DEFAULT_SOCKET_PATHS: &[&str] = &[
    "/tmp/cmux.sock",
    "/var/run/cmux.sock",
];

/// Spawner implementation that communicates with a running cmux instance.
pub struct CmuxSpawner {
    socket_path: String,
}

impl CmuxSpawner {
    /// Detect cmux availability and return a `CmuxSpawner` if found.
    pub fn detect() -> Option<Self> {
        // 1. Check environment variable
        if let Ok(path) = std::env::var("CMUX_SOCKET_PATH") {
            if std::path::Path::new(&path).exists() {
                return Some(CmuxSpawner { socket_path: path });
            }
        }
        // 2. Try known default paths
        for path in DEFAULT_SOCKET_PATHS {
            if std::path::Path::new(path).exists() {
                return Some(CmuxSpawner {
                    socket_path: path.to_string(),
                });
            }
        }
        None
    }

    /// Send a JSON-RPC call to cmux over the Unix socket.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .context("Failed to connect to cmux socket")?;

        let req = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: method.to_string(),
            params,
        };
        let payload = serde_json::to_string(&req)?;
        stream
            .write_all(payload.as_bytes())
            .await
            .context("Failed to write to cmux socket")?;
        stream
            .write_all(b"\n")
            .await?;

        // Read response
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.contains(&b'\n') {
                break;
            }
        }

        let resp: RpcResponse = serde_json::from_slice(&buf).context("Failed to parse cmux response")?;
        if let Some(err) = resp.error {
            anyhow::bail!("cmux error: {}", err);
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }
}

#[async_trait]
impl Spawner for CmuxSpawner {
    async fn create_workspace(&self, name: &str, cwd: &Path) -> Result<WorkspaceId> {
        // new-workspace, rename it, set cwd
        self.call(
            "new-workspace",
            json!({ "name": name, "cwd": cwd.to_string_lossy() }),
        )
        .await?;
        Ok(WorkspaceId(name.to_string()))
    }

    async fn create_split(
        &self,
        workspace: &WorkspaceId,
        direction: SplitDirection,
    ) -> Result<SurfaceId> {
        let dir = match direction {
            SplitDirection::Right => "right",
            SplitDirection::Down => "down",
        };
        let result = self
            .call(
                "new-split",
                json!({ "workspace": workspace.0, "direction": dir }),
            )
            .await?;
        let surface_id = result
            .get("surface_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("{}-split", workspace.0))
            .to_string();
        Ok(SurfaceId(surface_id))
    }

    async fn send_keys(&self, surface: &SurfaceId, keys: &str) -> Result<()> {
        self.call("send-keys", json!({ "surface": surface.0, "keys": keys }))
            .await?;
        Ok(())
    }

    async fn set_status(&self, workspace: &WorkspaceId, status: &StatusUpdate) -> Result<()> {
        self.call(
            "set-status",
            json!({
                "workspace": workspace.0,
                "text": status.text,
                "icon": status.icon,
                "color": status.color,
            }),
        )
        .await?;
        Ok(())
    }

    async fn set_progress(
        &self,
        workspace: &WorkspaceId,
        progress: f32,
        label: &str,
    ) -> Result<()> {
        self.call(
            "set-progress",
            json!({
                "workspace": workspace.0,
                "progress": progress,
                "label": label,
            }),
        )
        .await?;
        Ok(())
    }

    async fn log(&self, workspace: &WorkspaceId, level: LogLevel, message: &str) -> Result<()> {
        let level_str = match level {
            LogLevel::Info => "info",
            LogLevel::Success => "success",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
        };
        self.call(
            "log",
            json!({
                "workspace": workspace.0,
                "level": level_str,
                "message": message,
            }),
        )
        .await?;
        Ok(())
    }

    async fn notify(&self, title: &str, body: &str) -> Result<()> {
        self.call("notify", json!({ "title": title, "body": body }))
            .await?;
        Ok(())
    }

    async fn read_screen(&self, workspace: &WorkspaceId) -> Result<String> {
        let result = self
            .call("read-screen", json!({ "workspace": workspace.0 }))
            .await?;
        Ok(result
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    async fn destroy_workspace(&self, workspace: &WorkspaceId) -> Result<()> {
        self.call("destroy-workspace", json!({ "workspace": workspace.0 }))
            .await?;
        Ok(())
    }
}
