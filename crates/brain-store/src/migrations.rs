use anyhow::Result;
use rusqlite::{Connection, Transaction, TransactionBehavior};

const FTS_MIGRATION_VERSION: i64 = 5;

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
        "#,
    )?;
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

        CREATE TABLE IF NOT EXISTS note_imports (
            project_id TEXT NOT NULL,
            note_path TEXT NOT NULL,
            content_hash BLOB NOT NULL CHECK(length(content_hash) = 32),
            memory_id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            imported_at_ns INTEGER NOT NULL,
            PRIMARY KEY(project_id, note_path)
        );

        CREATE TABLE IF NOT EXISTS note_review_queue (
            review_id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id TEXT NOT NULL,
            note_path TEXT NOT NULL,
            content_hash BLOB NOT NULL CHECK(length(content_hash) = 32),
            reason TEXT NOT NULL,
            observed_at_ns INTEGER NOT NULL,
            resolved_at_ns INTEGER,
            UNIQUE(project_id, note_path, content_hash)
        );

        CREATE INDEX IF NOT EXISTS idx_note_review_unresolved
            ON note_review_queue(project_id, observed_at_ns DESC)
            WHERE resolved_at_ns IS NULL;

        CREATE TABLE IF NOT EXISTS global_preference_audit (
            audit_id TEXT PRIMARY KEY NOT NULL,
            note_path TEXT NOT NULL,
            content_hash BLOB NOT NULL CHECK(length(content_hash) = 32),
            preference_id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            action TEXT NOT NULL CHECK(action = 'explicit_promotion'),
            observed_at_ns INTEGER NOT NULL
        );

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

        CREATE TABLE IF NOT EXISTS sealed_segments (
            segment_id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT NOT NULL,
            manifest_path TEXT NOT NULL UNIQUE,
            data_path TEXT NOT NULL UNIQUE,
            first_event_id TEXT NOT NULL REFERENCES events(event_id),
            last_event_id TEXT NOT NULL REFERENCES events(event_id),
            event_count INTEGER NOT NULL CHECK(event_count > 0),
            occurred_min_ns INTEGER NOT NULL,
            occurred_max_ns INTEGER NOT NULL,
            compressed_sha256 BLOB NOT NULL CHECK(length(compressed_sha256) = 32),
            uncompressed_sha256 BLOB NOT NULL CHECK(length(uncompressed_sha256) = 32),
            compressed_bytes INTEGER NOT NULL,
            uncompressed_bytes INTEGER NOT NULL,
            published_at_ns INTEGER NOT NULL,
            UNIQUE(project_id, first_event_id, last_event_id)
        );

        CREATE INDEX IF NOT EXISTS idx_sealed_segments_project_time
            ON sealed_segments(project_id, occurred_min_ns, occurred_max_ns);

        CREATE TABLE IF NOT EXISTS event_segment_catalog (
            event_id TEXT PRIMARY KEY NOT NULL REFERENCES events(event_id),
            project_id TEXT NOT NULL,
            segment_id TEXT NOT NULL REFERENCES sealed_segments(segment_id),
            line_number INTEGER NOT NULL CHECK(line_number >= 0),
            raw_hash BLOB NOT NULL CHECK(length(raw_hash) = 32),
            search_text TEXT NOT NULL DEFAULT '',
            path TEXT NOT NULL DEFAULT ''
        );

        CREATE INDEX IF NOT EXISTS idx_event_segment_catalog_project_segment
            ON event_segment_catalog(project_id, segment_id, line_number);

        CREATE TABLE IF NOT EXISTS project_blob_refs (
            project_id TEXT NOT NULL,
            raw_sha256 BLOB NOT NULL CHECK(length(raw_sha256) = 32),
            mime TEXT NOT NULL,
            raw_bytes INTEGER NOT NULL,
            compressed_bytes INTEGER NOT NULL,
            created_at_ns INTEGER NOT NULL,
            PRIMARY KEY(project_id, raw_sha256)
        );

        CREATE TABLE IF NOT EXISTS provider_cache (
            provider TEXT NOT NULL,
            project_id TEXT NOT NULL,
            task_id TEXT,
            query_sha256 BLOB NOT NULL CHECK(length(query_sha256) = 32),
            config_sha256 BLOB NOT NULL CHECK(length(config_sha256) = 32),
            source_version TEXT NOT NULL,
            fetched_at_ns INTEGER NOT NULL,
            expires_at_ns INTEGER NOT NULL,
            items_json TEXT NOT NULL,
            PRIMARY KEY(provider, project_id, query_sha256, config_sha256, source_version)
        );

        CREATE INDEX IF NOT EXISTS idx_provider_cache_project_expiry
            ON provider_cache(project_id, provider, expires_at_ns);

        CREATE TABLE IF NOT EXISTS provider_prompt_state (
            project_id TEXT NOT NULL,
            harness TEXT NOT NULL,
            native_session_id TEXT NOT NULL,
            prompt_sha256 BLOB NOT NULL CHECK(length(prompt_sha256) = 32),
            claimed_at_ns INTEGER NOT NULL,
            PRIMARY KEY(project_id, harness, native_session_id)
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

        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (1, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (2, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (3, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (4, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (6, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (7, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (8, datetime('now'));
        INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (9, datetime('now'));
        "#,
    )?;
    apply_fts_data_migration(connection)?;
    ensure_column(
        connection,
        "events",
        "archived",
        "ALTER TABLE events ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "event_segment_catalog",
        "search_text",
        "ALTER TABLE event_segment_catalog ADD COLUMN search_text TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        connection,
        "event_segment_catalog",
        "path",
        "ALTER TABLE event_segment_catalog ADD COLUMN path TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn apply_fts_data_migration(connection: &Connection) -> Result<()> {
    if migration_applied(connection, FTS_MIGRATION_VERSION)? {
        return Ok(());
    }
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    if !migration_applied(&transaction, FTS_MIGRATION_VERSION)? {
        transaction.execute_batch(
            r#"
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

            INSERT INTO schema_migrations(version, applied_at)
                VALUES (5, datetime('now'));
            "#,
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn migration_applied(connection: &Connection, version: i64) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
        [version],
        |row| row.get(0),
    )?)
}

fn ensure_column(
    connection: &rusqlite::Connection,
    table: &str,
    column: &str,
    statement: &str,
) -> anyhow::Result<()> {
    let mut query = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = query
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !names.iter().any(|name| name == column) {
        connection.execute_batch(statement)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::migrate;

    #[test]
    fn reopening_a_current_ledger_never_replays_the_fts_data_migration() {
        let connection = Connection::open_in_memory().expect("open migration fixture");
        migrate(&connection).expect("create current schema");
        connection
            .execute_batch(
                r#"
                INSERT INTO events(
                    event_id, project_id, worktree_id, task_id, harness,
                    native_session_id, native_turn_id, event_type, occurred_at_ns,
                    observed_at_ns, source_locator, source_offset, source_schema,
                    raw_hash, idempotency_key, git_head, git_branch,
                    payload_json, raw_json
                ) VALUES (
                    '00000000-0000-7000-8000-000000000001',
                    '00000000-0000-7000-8000-000000000002',
                    '00000000-0000-7000-8000-000000000003',
                    NULL, 'codex', 'session-1', 'turn-1', 'user.prompted',
                    1, 1, 'fixture.jsonl', 1, 'fixture:v1',
                    zeroblob(32), randomblob(32), NULL, NULL,
                    '{"message":"migration sentinel"}',
                    '{"message":"migration sentinel"}'
                );
                INSERT INTO event_search(event_search) VALUES('delete-all');
                "#,
            )
            .expect("seed then clear derived FTS row");
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM event_search", [], |row| {
                    row.get::<_, u64>(0)
                })
                .expect("count cleared FTS rows"),
            0
        );

        migrate(&connection).expect("reopen current schema");

        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM event_search", [], |row| {
                    row.get::<_, u64>(0)
                })
                .expect("count FTS rows after reopen"),
            0,
            "a recorded data migration must not scan and repair FTS during normal startup"
        );
    }
}
