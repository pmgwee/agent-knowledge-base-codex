#![forbid(unsafe_code)]

mod lease;
mod task;

use std::path::Path;

use anyhow::Result;
use brain_domain::ProjectId;
use rusqlite::Connection;

pub use lease::{CoordinationEvent, LeaseError, SessionIdentity, WriterLease};
pub use task::{TaskRecord, TaskStatus};

pub const DEFAULT_LEASE_DURATION: time::Duration = time::Duration::minutes(30);
pub const RENEWAL_INTERVAL: time::Duration = time::Duration::minutes(5);

pub struct CoordinationStore {
    pub(crate) connection: Connection,
    pub(crate) project_id: ProjectId,
}

impl CoordinationStore {
    pub fn open(path: impl AsRef<Path>, project_id: ProjectId) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "busy_timeout", 1_000_i64)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        Ok(Self {
            connection,
            project_id,
        })
    }

    pub fn open_in_memory(project_id: ProjectId) -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", true)?;
        migrate(&connection)?;
        Ok(Self {
            connection,
            project_id,
        })
    }
}

fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS coordination_tasks (
            task_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            worktree_id TEXT NOT NULL,
            title TEXT NOT NULL,
            worktree_path TEXT,
            branch TEXT,
            status TEXT NOT NULL,
            created_at_ns INTEGER NOT NULL,
            closed_at_ns INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_coordination_tasks_project_status
            ON coordination_tasks(project_id, status, created_at_ns DESC);
        CREATE TABLE IF NOT EXISTS writer_leases (
            task_id TEXT PRIMARY KEY NOT NULL REFERENCES coordination_tasks(task_id),
            project_id TEXT NOT NULL,
            worktree_id TEXT NOT NULL UNIQUE,
            owner_harness TEXT NOT NULL,
            owner_session TEXT NOT NULL,
            acquired_at_ns INTEGER NOT NULL,
            renewed_at_ns INTEGER NOT NULL,
            expires_at_ns INTEGER NOT NULL,
            generation INTEGER NOT NULL CHECK(generation > 0)
        );
        CREATE INDEX IF NOT EXISTS idx_writer_leases_project_expiry
            ON writer_leases(project_id, expires_at_ns);
        CREATE TABLE IF NOT EXISTS coordination_events (
            event_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            worktree_id TEXT NOT NULL,
            action TEXT NOT NULL,
            owner_harness TEXT,
            owner_session TEXT,
            prior_owner_harness TEXT,
            prior_owner_session TEXT,
            generation INTEGER,
            occurred_at_ns INTEGER NOT NULL,
            details_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_coordination_events_project_time
            ON coordination_events(project_id, occurred_at_ns DESC);
        CREATE TABLE IF NOT EXISTS path_claims (
            claim_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            task_id TEXT NOT NULL REFERENCES coordination_tasks(task_id),
            worktree_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            normalized_value TEXT NOT NULL,
            display_value TEXT NOT NULL,
            symbol TEXT,
            created_at_ns INTEGER NOT NULL,
            released_at_ns INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_path_claims_project_active
            ON path_claims(project_id, released_at_ns, normalized_value);
        "#,
    )?;
    Ok(())
}

pub(crate) fn timestamp_ns(value: time::OffsetDateTime) -> Result<i64> {
    Ok(i64::try_from(value.unix_timestamp_nanos())?)
}

pub(crate) fn from_ns(value: i64) -> Result<time::OffsetDateTime> {
    Ok(time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(value),
    )?)
}
