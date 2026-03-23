use anyhow::Result;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Database handle with single-writer + reader pool pattern.
///
/// - All mutations go through `writer` (Mutex<Connection>) with BEGIN IMMEDIATE.
/// - Concurrent reads use the `readers` pool.
pub struct Db {
    /// Single writer connection protected by mutex — prevents write-write contention.
    pub writer: Mutex<Connection>,
    /// Reader connection pool for concurrent read queries.
    pub readers: Pool<SqliteConnectionManager>,
}

impl Db {
    /// Open the database, configure WAL mode, and initialize the schema.
    pub fn open(path: &Path) -> Result<Self> {
        // Writer connection
        let writer = Connection::open(path)?;
        schema::configure_connection(&writer)?;

        // Reader pool — each connection gets WAL + foreign keys enabled on open
        let manager = SqliteConnectionManager::file(path)
            .with_init(|conn| schema::configure_connection_raw(conn));
        let readers = Pool::builder().max_size(8).build(manager)?;

        let db = Self {
            writer: Mutex::new(writer),
            readers,
        };

        // Initialize schema on writer
        {
            let conn = db.writer.lock().unwrap();
            schema::init_schema(&conn)?;
        }

        Ok(db)
    }
}

pub mod file_changes;
pub mod member;
pub mod message;
pub mod schema;
pub mod task;
pub mod task_dep;
pub mod team;
pub mod worktree;
