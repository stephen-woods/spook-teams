use std::sync::Arc;
use anyhow::Result;
use clap::Parser;
use tracing::info;

mod config;
mod db;
mod server;
mod agent;
mod team;
mod task;
mod message;
mod worktree;
mod opencode;
mod event;
mod bridge;
mod spawner;

use config::Config;
use db::Db;
use server::run_server;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();

    // Initialize tracing
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!(
        port = config.port,
        db_path = %config.db_path.display(),
        project_path = %config.project_path.display(),
        "Starting spook-teams MCP server"
    );

    // Initialize database
    let db = Arc::new(Db::open(&config.db_path)?);

    // Run the server (stdio + HTTP)
    run_server(config, db).await
}
