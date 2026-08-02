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

        CREATE TABLE IF NOT EXISTS schema_drifts (
            diagnostic_id TEXT PRIMARY KEY NOT NULL,
            source_id TEXT NOT NULL,
            expected_fingerprint TEXT NOT NULL,
            observed_fingerprint TEXT NOT NULL,
            cursor_json TEXT NOT NULL,
            sample_hash BLOB NOT NULL CHECK(length(sample_hash) = 32),
            reason TEXT NOT NULL,
            observed_at_ns INTEGER NOT NULL,
            resolved_at_ns INTEGER
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_schema_drifts_one_active_source
            ON schema_drifts(source_id)
            WHERE resolved_at_ns IS NULL;
        CREATE INDEX IF NOT EXISTS idx_schema_drifts_observed
            ON schema_drifts(observed_at_ns DESC);

        CREATE TABLE IF NOT EXISTS memory_records (
            memory_id TEXT PRIMARY KEY NOT NULL,
            scope TEXT NOT NULL CHECK(scope = 'project'),
            project_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            projection_path TEXT NOT NULL UNIQUE,
            created_at_ns INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS memory_versions (
            version_id TEXT PRIMARY KEY NOT NULL,
            memory_id TEXT NOT NULL REFERENCES memory_records(memory_id),
            version_number INTEGER NOT NULL,
            worktree_id TEXT,
            task_id TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            valid_from_ns INTEGER NOT NULL,
            valid_to_ns INTEGER,
            recorded_at_ns INTEGER NOT NULL,
            confidence REAL NOT NULL CHECK(confidence >= 0.0 AND confidence <= 1.0),
            authority TEXT NOT NULL,
            status TEXT NOT NULL,
            UNIQUE(memory_id, version_number)
        );

        CREATE INDEX IF NOT EXISTS idx_memory_versions_current
            ON memory_versions(memory_id, version_number DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_versions_validity
            ON memory_versions(valid_from_ns, valid_to_ns);

        CREATE TABLE IF NOT EXISTS memory_evidence (
            version_id TEXT NOT NULL REFERENCES memory_versions(version_id),
            event_id TEXT NOT NULL REFERENCES events(event_id),
            PRIMARY KEY(version_id, event_id)
        );

        CREATE TABLE IF NOT EXISTS memory_supersession (
            version_id TEXT NOT NULL REFERENCES memory_versions(version_id),
            superseded_version_id TEXT NOT NULL REFERENCES memory_versions(version_id),
            PRIMARY KEY(version_id, superseded_version_id)
        );

        CREATE TABLE IF NOT EXISTS global_preferences (
            preference_id TEXT NOT NULL,
            version_id TEXT PRIMARY KEY NOT NULL,
            version_number INTEGER NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            valid_from_ns INTEGER NOT NULL,
            valid_to_ns INTEGER,
            recorded_at_ns INTEGER NOT NULL,
            confidence REAL NOT NULL CHECK(confidence >= 0.0 AND confidence <= 1.0),
            authority TEXT NOT NULL,
            status TEXT NOT NULL,
            projection_path TEXT NOT NULL,
            UNIQUE(preference_id, version_number)
        );

        CREATE TABLE IF NOT EXISTS projection_state (
            projection_path TEXT PRIMARY KEY NOT NULL,
            memory_id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            content_hash BLOB,
            projected_at_ns INTEGER,
            status TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS consolidation_jobs (
            job_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            first_event_id TEXT NOT NULL REFERENCES events(event_id),
            last_event_id TEXT NOT NULL REFERENCES events(event_id),
            reason TEXT NOT NULL,
            idempotency_key BLOB NOT NULL UNIQUE CHECK(length(idempotency_key) = 32),
            status TEXT NOT NULL,
            attempt INTEGER NOT NULL DEFAULT 0,
            available_at_ns INTEGER NOT NULL,
            lease_owner TEXT,
            lease_until_ns INTEGER,
            last_error TEXT,
            created_at_ns INTEGER NOT NULL,
            completed_at_ns INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_consolidation_jobs_available
            ON consolidation_jobs(project_id, status, available_at_ns, lease_until_ns);

        CREATE TABLE IF NOT EXISTS redaction_manifests (
            job_id TEXT NOT NULL REFERENCES consolidation_jobs(job_id),
            category TEXT NOT NULL,
            token_hash BLOB NOT NULL CHECK(length(token_hash) = 32),
            PRIMARY KEY(job_id, category, token_hash)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS event_search USING fts5(
            scope_token,
            content,
            path,
            task_label,
            aliases,
            content=''
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS memory_search USING fts5(
            scope_token,
            title,
            content,
            path,
            task_label,
            aliases,
            content=''
        );

        CREATE TRIGGER IF NOT EXISTS event_search_after_insert
        AFTER INSERT ON events BEGIN
            INSERT INTO event_search(
                rowid, scope_token, content, path, task_label, aliases
            ) VALUES (
                new.rowid,
                'p' || replace(new.project_id, '-', ''),
                new.event_type || ' ' || new.payload_json,
                new.source_locator || ' ' ||
                    coalesce(json_extract(new.payload_json, '$.path'), '') || ' ' ||
                    coalesce(json_extract(new.payload_json, '$.file_path'), ''),
                coalesce(new.task_id, ''),
                new.native_session_id || ' ' || coalesce(new.native_turn_id, '') || ' ' ||
                    coalesce(new.git_head, '') || ' ' || coalesce(new.git_branch, '')
            );
        END;

        CREATE TRIGGER IF NOT EXISTS memory_search_after_insert
        AFTER INSERT ON memory_versions BEGIN
            INSERT INTO memory_search(
                rowid, scope_token, title, content, path, task_label, aliases
            ) VALUES (
                new.rowid,
                'p' || replace(
                    (SELECT project_id FROM memory_records WHERE memory_id = new.memory_id),
                    '-', ''
                ),
                new.title,
                new.content,
                (SELECT projection_path FROM memory_records WHERE memory_id = new.memory_id),
                coalesce(new.task_id, ''),
                new.memory_id || ' ' || new.version_id || ' ' ||
                    (SELECT kind FROM memory_records WHERE memory_id = new.memory_id)
            );
        END;

        INSERT INTO event_search(rowid, scope_token, content, path, task_label, aliases)
        SELECT
            e.rowid,
            'p' || replace(e.project_id, '-', ''),
            e.event_type || ' ' || e.payload_json,
            e.source_locator || ' ' ||
                coalesce(json_extract(e.payload_json, '$.path'), '') || ' ' ||
                coalesce(json_extract(e.payload_json, '$.file_path'), ''),
            coalesce(e.task_id, ''),
            e.native_session_id || ' ' || coalesce(e.native_turn_id, '') || ' ' ||
                coalesce(e.git_head, '') || ' ' || coalesce(e.git_branch, '')
        FROM events e
        WHERE NOT EXISTS (SELECT 1 FROM event_search WHERE rowid = e.rowid);

        INSERT INTO memory_search(rowid, scope_token, title, content, path, task_label, aliases)
        SELECT
            v.rowid,
            'p' || replace(r.project_id, '-', ''),
            v.title,
            v.content,
            r.projection_path,
            coalesce(v.task_id, ''),
            v.memory_id || ' ' || v.version_id || ' ' || r.kind
        FROM memory_versions v
        JOIN memory_records r ON r.memory_id = v.memory_id
        WHERE NOT EXISTS (SELECT 1 FROM memory_search WHERE rowid = v.rowid);

        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (1, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (2, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (3, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (4, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (5, datetime('now'));
        "#,
    )?;
    Ok(())
}
