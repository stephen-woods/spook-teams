use clap::Parser;
use std::path::PathBuf;

/// spook-teams: MCP server for coordinating teams of OpenCode agents
#[derive(Parser, Debug, Clone)]
#[command(name = "spook-teams", version, about)]
pub struct Config {
    /// HTTP port for worker agent connections
    #[arg(long, default_value = "3001")]
    pub port: u16,

    /// Path to the SQLite database file
    #[arg(long, default_value = "spook-teams.db")]
    pub db_path: PathBuf,

    /// Path to the project root directory
    #[arg(long, default_value = ".")]
    pub project_path: PathBuf,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Base port for spawning OpenCode serve instances
    #[arg(long, default_value = "4097")]
    pub agent_base_port: u16,
}
