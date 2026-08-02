use anyhow::Result;
use rusqlite::Connection;

pub(crate) fn configure(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "busy_timeout", 1_000_i64)?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    Ok(())
}

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS events (
            event_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            worktree_id TEXT NOT NULL,
            task_id TEXT,
            harness TEXT NOT NULL,
            native_session_id TEXT NOT NULL,
            native_turn_id TEXT,
            event_type TEXT NOT NULL,
            occurred_at_ns INTEGER NOT NULL,
            observed_at_ns INTEGER NOT NULL,
            source_locator TEXT NOT NULL,
            source_offset INTEGER NOT NULL,
            source_schema TEXT NOT NULL,
            raw_hash BLOB NOT NULL CHECK(length(raw_hash) = 32),
            idempotency_key BLOB NOT NULL CHECK(length(idempotency_key) = 32),
            git_head TEXT,
            git_branch TEXT,
            payload_json TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            UNIQUE(idempotency_key)
        );

        CREATE INDEX IF NOT EXISTS idx_events_project_occurred
            ON events(project_id, occurred_at_ns DESC);
        CREATE INDEX IF NOT EXISTS idx_events_session_source
            ON events(native_session_id, source_offset);
        CREATE INDEX IF NOT EXISTS idx_events_type_occurred
            ON events(event_type, occurred_at_ns DESC);

        CREATE TABLE IF NOT EXISTS source_cursors (
            source_id TEXT PRIMARY KEY NOT NULL,
            cursor_json TEXT NOT NULL,
            updated_at_ns INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS quarantine (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id TEXT NOT NULL,
            source_locator TEXT NOT NULL,
            source_offset INTEGER NOT NULL,
            raw_hash BLOB NOT NULL,
            error TEXT NOT NULL,
            observed_at_ns INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS capture_gaps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id TEXT NOT NULL,
            expected_cursor_json TEXT NOT NULL,
            observed_cursor_json TEXT,
            reason TEXT NOT NULL,
            observed_at_ns INTEGER NOT NULL,
            resolved_at_ns INTEGER
        );

        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (1, datetime('now'));
        "#,
    )?;
    Ok(())
}
