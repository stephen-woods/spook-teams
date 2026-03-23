use anyhow::Result;
use rusqlite::Connection;

/// Configure a connection with WAL mode and pragmas.
pub fn configure_connection(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(())
}

/// Same as configure_connection but takes a raw rusqlite::Connection (for r2d2 init).
pub fn configure_connection_raw(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )
}

/// Initialize all tables in the database.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS teams (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    change_name  TEXT NOT NULL,
    project_path TEXT NOT NULL,
    tasks_path   TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS members (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    name         TEXT NOT NULL,
    role         TEXT NOT NULL DEFAULT 'worker',
    status       TEXT NOT NULL DEFAULT 'pending_spawn',
    session_id   TEXT,
    port         INTEGER,
    worktree_id  TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    source_id    TEXT NOT NULL,
    title        TEXT NOT NULL,
    description  TEXT,
    status       TEXT NOT NULL DEFAULT 'pending',
    owner_id     TEXT REFERENCES members(id),
    section      TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id      TEXT NOT NULL REFERENCES tasks(id),
    depends_on   TEXT NOT NULL REFERENCES tasks(id),
    PRIMARY KEY (task_id, depends_on)
);

CREATE TABLE IF NOT EXISTS messages (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    sender_id    TEXT NOT NULL,
    recipient    TEXT NOT NULL,
    topic        TEXT,
    message_type TEXT NOT NULL DEFAULT 'text',
    body         TEXT NOT NULL,
    metadata     TEXT,
    is_read      INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worktrees (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    member_id    TEXT NOT NULL REFERENCES members(id),
    path         TEXT NOT NULL,
    branch       TEXT NOT NULL,
    base_commit  TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS file_changes (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    member_id    TEXT NOT NULL REFERENCES members(id),
    file_path    TEXT NOT NULL,
    change_type  TEXT NOT NULL,
    commit_hash  TEXT,
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_events (
    id           TEXT PRIMARY KEY,
    team_id      TEXT NOT NULL REFERENCES teams(id),
    member_id    TEXT NOT NULL REFERENCES members(id),
    event_type   TEXT NOT NULL,
    payload      TEXT,
    created_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_members_team ON members(team_id);
CREATE INDEX IF NOT EXISTS idx_tasks_team ON tasks(team_id);
CREATE INDEX IF NOT EXISTS idx_tasks_owner ON tasks(owner_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient, is_read);
CREATE INDEX IF NOT EXISTS idx_messages_team ON messages(team_id);
CREATE INDEX IF NOT EXISTS idx_file_changes_team ON file_changes(team_id, member_id);
CREATE INDEX IF NOT EXISTS idx_agent_events_member ON agent_events(member_id);
"#;
