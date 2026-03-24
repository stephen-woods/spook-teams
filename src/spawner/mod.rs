use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub mod cmux;



// ── Types ─────────────────────────────────────────────────────────────────────

/// Opaque workspace identifier (maps to a cmux workspace name).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub String);

/// Opaque pane/surface identifier (maps to a cmux surface id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SurfaceId(pub String);

#[derive(Debug, Clone)]
pub enum SplitDirection {
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    pub text: String,
    pub icon: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

// ── Spawner trait ─────────────────────────────────────────────────────────────

/// Abstracts terminal multiplexer operations for agent workspace management.
/// Only `CmuxSpawner` is implemented; `HeadlessSpawner` is the no-op fallback.
#[async_trait]
pub trait Spawner: Send + Sync {
    /// Create a new workspace (tab/window) with `cwd` as the working directory.
    async fn create_workspace(
        &self,
        name: &str,
        cwd: &Path,
    ) -> Result<WorkspaceId>;

    /// Create a split pane in the given workspace.
    async fn create_split(
        &self,
        workspace: &WorkspaceId,
        direction: SplitDirection,
    ) -> Result<SurfaceId>;

    /// Send key strokes / a command to a pane.
    async fn send_keys(&self, surface: &SurfaceId, keys: &str) -> Result<()>;

    /// Update sidebar status for a workspace.
    async fn set_status(
        &self,
        workspace: &WorkspaceId,
        status: &StatusUpdate,
    ) -> Result<()>;

    /// Update sidebar progress bar (0.0–1.0) and label.
    async fn set_progress(
        &self,
        workspace: &WorkspaceId,
        progress: f32,
        label: &str,
    ) -> Result<()>;

    /// Log a message to the sidebar log.
    async fn log(
        &self,
        workspace: &WorkspaceId,
        level: LogLevel,
        message: &str,
    ) -> Result<()>;

    /// Send a desktop notification.
    async fn notify(&self, title: &str, body: &str) -> Result<()>;

    /// Read the screen content (for crash context capture).
    async fn read_screen(&self, workspace: &WorkspaceId) -> Result<String>;
}

// ── HeadlessSpawner ───────────────────────────────────────────────────────────

/// No-op spawner used when cmux is not available.
/// All operations succeed but produce no UI effects.
pub struct HeadlessSpawner;

#[async_trait]
impl Spawner for HeadlessSpawner {
    async fn create_workspace(&self, name: &str, _cwd: &Path) -> Result<WorkspaceId> {
        tracing::debug!("HeadlessSpawner: create_workspace({})", name);
        Ok(WorkspaceId(name.to_string()))
    }

    async fn create_split(
        &self,
        workspace: &WorkspaceId,
        _direction: SplitDirection,
    ) -> Result<SurfaceId> {
        tracing::debug!("HeadlessSpawner: create_split({})", workspace.0);
        Ok(SurfaceId(format!("{}-split", workspace.0)))
    }

    async fn send_keys(&self, surface: &SurfaceId, keys: &str) -> Result<()> {
        tracing::debug!("HeadlessSpawner: send_keys({}, {:?})", surface.0, keys);
        Ok(())
    }

    async fn set_status(&self, workspace: &WorkspaceId, status: &StatusUpdate) -> Result<()> {
        tracing::debug!("HeadlessSpawner: set_status({}, {})", workspace.0, status.text);
        Ok(())
    }

    async fn set_progress(
        &self,
        workspace: &WorkspaceId,
        progress: f32,
        label: &str,
    ) -> Result<()> {
        tracing::debug!(
            "HeadlessSpawner: set_progress({}, {:.2}, {})",
            workspace.0,
            progress,
            label
        );
        Ok(())
    }

    async fn log(&self, workspace: &WorkspaceId, level: LogLevel, message: &str) -> Result<()> {
        tracing::debug!(
            "HeadlessSpawner: log({}, {:?}, {})",
            workspace.0,
            level,
            message
        );
        Ok(())
    }

    async fn notify(&self, title: &str, body: &str) -> Result<()> {
        tracing::debug!("HeadlessSpawner: notify({:?}, {:?})", title, body);
        Ok(())
    }

    async fn read_screen(&self, workspace: &WorkspaceId) -> Result<String> {
        tracing::debug!("HeadlessSpawner: read_screen({})", workspace.0);
        Ok(String::new())
    }
}

/// Detect whether cmux is available and return the appropriate spawner.
pub fn detect_spawner() -> Box<dyn Spawner> {
    if let Some(spawner) = cmux::CmuxSpawner::detect() {
        tracing::info!("cmux detected, using CmuxSpawner");
        Box::new(spawner)
    } else {
        tracing::warn!("cmux not detected, running in headless mode");
        Box::new(HeadlessSpawner)
    }
}
