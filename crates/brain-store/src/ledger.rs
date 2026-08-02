use std::path::Path;

use anyhow::{Result, bail};
use brain_domain::{
    CaptureGapRecord, EventBatch, EventType, Harness, NormalizedEvent, ProjectId,
    QuarantinedRecord, SchemaDriftRecord, SourceCursor, WorktreeId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::cursor::{load_cursor, save_cursor, timestamp_ns};
use crate::migrations::{configure, migrate};

pub struct EventLedger {
    pub(crate) connection: Connection,
    pub(crate) project_scope: ProjectId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendResult {
    pub inserted: usize,
    pub quarantined: usize,
    pub capture_gaps: usize,
}

#[derive(Clone, Debug)]
pub struct StoredEvent {
    pub event_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub task_id: Option<uuid::Uuid>,
    pub harness: Harness,
    pub native_session_id: String,
    pub event_type: EventType,
    pub occurred_at: time::OffsetDateTime,
    pub observed_at: time::OffsetDateTime,
    pub source_locator: String,
    pub source_offset: i64,
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
}

impl EventLedger {
    pub const fn project_id(&self) -> ProjectId {
        self.project_scope
    }

    pub fn open(path: impl AsRef<Path>, project_id: ProjectId) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path)?, project_id)
    }

    pub fn open_in_memory(project_id: ProjectId) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, project_id)
    }

    fn from_connection(connection: Connection, project_scope: ProjectId) -> Result<Self> {
        configure(&connection)?;
        migrate(&connection)?;
        Ok(Self {
            connection,
            project_scope,
        })
    }

    pub fn append_batch(&mut self, batch: &EventBatch) -> Result<AppendResult> {
        if batch
            .events
            .iter()
            .any(|event| event.project_id != self.project_scope)
        {
            bail!(
                "event batch violates project scope {}",
                self.project_scope.0
            );
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut inserted = 0;
        for event in &batch.events {
            inserted += insert_event(&transaction, event)?;
        }
        let mut quarantined = 0;
        for record in &batch.quarantined {
            quarantined += insert_quarantine(&transaction, &batch.source_id, record)?;
        }
        let mut capture_gaps = 0;
        for gap in &batch.capture_gaps {
            capture_gaps += insert_capture_gap(&transaction, &batch.source_id, gap)?;
        }
        save_cursor(&transaction, &batch.source_id, &batch.next_cursor)?;
        transaction.commit()?;
        Ok(AppendResult {
            inserted,
            quarantined,
            capture_gaps,
        })
    }

    pub fn event_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?)
    }

    pub fn event_count_by_harness(&self, harness: brain_domain::Harness) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM events WHERE harness = ?1",
            [harness.as_str()],
            |row| row.get(0),
        )?)
    }

    pub fn latest_event_at(&self) -> Result<Option<time::OffsetDateTime>> {
        let timestamp =
            self.connection
                .query_row("SELECT MAX(occurred_at_ns) FROM events", [], |row| {
                    row.get::<_, Option<i64>>(0)
                })?;
        timestamp
            .map(|value| time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value)))
            .transpose()
            .map_err(Into::into)
    }

    pub fn quarantine_count(&self, source_id: &str) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM quarantine WHERE source_id = ?1",
            [source_id],
            |row| row.get(0),
        )?)
    }

    pub fn unresolved_capture_gap_count(&self, source_id: &str) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM capture_gaps WHERE source_id = ?1 AND resolved_at_ns IS NULL",
            [source_id],
            |row| row.get(0),
        )?)
    }

    pub fn record_schema_drift(&mut self, record: &SchemaDriftRecord) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = transaction.execute(
            r#"
            UPDATE schema_drifts SET
                expected_fingerprint = ?2,
                observed_fingerprint = ?3,
                cursor_json = ?4,
                sample_hash = ?5,
                reason = ?6,
                observed_at_ns = ?7
            WHERE source_id = ?1 AND resolved_at_ns IS NULL
            "#,
            params![
                record.source_id,
                record.expected_fingerprint,
                record.observed_fingerprint,
                serde_json::to_string(&record.cursor)?,
                record.sample_hash.as_slice(),
                record.reason,
                timestamp_ns(record.observed_at)?,
            ],
        )?;
        if updated == 0 {
            transaction.execute(
                r#"
                INSERT INTO schema_drifts(
                    diagnostic_id, source_id, expected_fingerprint,
                    observed_fingerprint, cursor_json, sample_hash,
                    reason, observed_at_ns, resolved_at_ns
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)
                "#,
                params![
                    record.diagnostic_id.to_string(),
                    record.source_id,
                    record.expected_fingerprint,
                    record.observed_fingerprint,
                    serde_json::to_string(&record.cursor)?,
                    record.sample_hash.as_slice(),
                    record.reason,
                    timestamp_ns(record.observed_at)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn active_schema_drift(&self, source_id: &str) -> Result<Option<SchemaDriftRecord>> {
        self.connection
            .query_row(
                r#"
                SELECT diagnostic_id, source_id, expected_fingerprint,
                       observed_fingerprint, cursor_json, sample_hash,
                       reason, observed_at_ns, resolved_at_ns
                FROM schema_drifts
                WHERE source_id = ?1 AND resolved_at_ns IS NULL
                "#,
                [source_id],
                parse_schema_drift,
            )
            .optional()?
            .map(parse_schema_drift_record)
            .transpose()
    }

    pub fn active_schema_drift_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM schema_drifts WHERE resolved_at_ns IS NULL",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn resolve_schema_drift(
        &mut self,
        source_id: &str,
        resolved_at: time::OffsetDateTime,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            r#"
            UPDATE schema_drifts
            SET resolved_at_ns = ?2
            WHERE source_id = ?1 AND resolved_at_ns IS NULL
            "#,
            params![source_id, timestamp_ns(resolved_at)?],
        )? > 0)
    }

    pub fn schema_drifts(&self) -> Result<Vec<SchemaDriftRecord>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT diagnostic_id, source_id, expected_fingerprint,
                   observed_fingerprint, cursor_json, sample_hash,
                   reason, observed_at_ns, resolved_at_ns
            FROM schema_drifts
            ORDER BY observed_at_ns DESC, diagnostic_id DESC
            "#,
        )?;
        let rows = statement.query_map([], parse_schema_drift)?;
        rows.map(|row| parse_schema_drift_record(row?)).collect()
    }

    pub fn cursor(&self, source_id: &str) -> Result<SourceCursor> {
        load_cursor(&self.connection, source_id)
    }

    pub fn explain_project_time_query(&self, project_id: ProjectId) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            r#"
            EXPLAIN QUERY PLAN
            SELECT event_id
            FROM events
            WHERE project_id = ?1
            ORDER BY occurred_at_ns DESC
            LIMIT 50
            "#,
        )?;
        let rows = statement.query_map([project_id.0.to_string()], |row| row.get(3))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn raw_contains(&self, needle: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE instr(raw_json, ?1) > 0)",
            [needle],
            |row| row.get(0),
        )?)
    }

    pub fn recent_events(&self, project_id: ProjectId, limit: usize) -> Result<Vec<StoredEvent>> {
        if project_id != self.project_scope {
            bail!(
                "project {} cannot query ledger scoped to {}",
                project_id.0,
                self.project_scope.0
            );
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bounded_limit = i64::try_from(limit.min(500))?;
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, worktree_id, task_id, harness, native_session_id, event_type,
                   occurred_at_ns, observed_at_ns, source_locator, source_offset,
                   git_head, git_branch, payload_json, raw_json
            FROM events
            WHERE project_id = ?1
            ORDER BY occurred_at_ns DESC, observed_at_ns DESC, source_offset DESC
            LIMIT ?2
            "#,
        )?;
        let rows =
            statement.query_map(params![project_id.0.to_string(), bounded_limit], |row| {
                Ok(RawStoredEvent {
                    event_id: row.get(0)?,
                    worktree_id: row.get(1)?,
                    task_id: row.get(2)?,
                    harness: row.get(3)?,
                    native_session_id: row.get(4)?,
                    event_type: row.get(5)?,
                    occurred_at_ns: row.get(6)?,
                    observed_at_ns: row.get(7)?,
                    source_locator: row.get(8)?,
                    source_offset: row.get(9)?,
                    git_head: row.get(10)?,
                    git_branch: row.get(11)?,
                    payload_json: row.get(12)?,
                    raw_json: row.get(13)?,
                })
            })?;
        let mut events = Vec::new();
        for row in rows {
            if let Some(event) = row?.parse(project_id) {
                events.push(event);
            }
        }
        Ok(events)
    }

    pub fn events_between(
        &self,
        first_event_id: uuid::Uuid,
        last_event_id: uuid::Uuid,
    ) -> Result<Vec<StoredEvent>> {
        let first_row: i64 = self.connection.query_row(
            "SELECT rowid FROM events WHERE event_id = ?1 AND project_id = ?2",
            params![first_event_id.to_string(), self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        let last_row: i64 = self.connection.query_row(
            "SELECT rowid FROM events WHERE event_id = ?1 AND project_id = ?2",
            params![last_event_id.to_string(), self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        let (start, end) = if first_row <= last_row {
            (first_row, last_row)
        } else {
            (last_row, first_row)
        };
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, worktree_id, task_id, harness, native_session_id, event_type,
                   occurred_at_ns, observed_at_ns, source_locator, source_offset,
                   git_head, git_branch, payload_json, raw_json
            FROM events
            WHERE project_id = ?1 AND rowid BETWEEN ?2 AND ?3
            ORDER BY rowid ASC
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_scope.0.to_string(), start, end],
            |row| {
                Ok(RawStoredEvent {
                    event_id: row.get(0)?,
                    worktree_id: row.get(1)?,
                    task_id: row.get(2)?,
                    harness: row.get(3)?,
                    native_session_id: row.get(4)?,
                    event_type: row.get(5)?,
                    occurred_at_ns: row.get(6)?,
                    observed_at_ns: row.get(7)?,
                    source_locator: row.get(8)?,
                    source_offset: row.get(9)?,
                    git_head: row.get(10)?,
                    git_branch: row.get(11)?,
                    payload_json: row.get(12)?,
                    raw_json: row.get(13)?,
                })
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            if let Some(event) = row?.parse(self.project_scope) {
                events.push(event);
            }
        }
        Ok(events)
    }
}

type RawSchemaDrift = (
    String,
    String,
    String,
    String,
    String,
    Vec<u8>,
    String,
    i64,
    Option<i64>,
);

fn parse_schema_drift(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSchemaDrift> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn parse_schema_drift_record(raw: RawSchemaDrift) -> Result<SchemaDriftRecord> {
    let sample_hash: [u8; 32] = raw
        .5
        .try_into()
        .map_err(|_| anyhow::anyhow!("stored schema drift sample hash is not 32 bytes"))?;
    Ok(SchemaDriftRecord {
        diagnostic_id: uuid::Uuid::parse_str(&raw.0)?,
        source_id: raw.1,
        expected_fingerprint: raw.2,
        observed_fingerprint: raw.3,
        cursor: serde_json::from_str(&raw.4)?,
        sample_hash,
        reason: raw.6,
        observed_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(raw.7))?,
        resolved_at: raw
            .8
            .map(|value| time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value)))
            .transpose()?,
    })
}

struct RawStoredEvent {
    event_id: String,
    worktree_id: String,
    task_id: Option<String>,
    harness: String,
    native_session_id: String,
    event_type: String,
    occurred_at_ns: i64,
    observed_at_ns: i64,
    source_locator: String,
    source_offset: i64,
    git_head: Option<String>,
    git_branch: Option<String>,
    payload_json: String,
    raw_json: String,
}

impl RawStoredEvent {
    fn parse(self, project_id: ProjectId) -> Option<StoredEvent> {
        Some(StoredEvent {
            event_id: uuid::Uuid::parse_str(&self.event_id).ok()?,
            project_id,
            worktree_id: WorktreeId(uuid::Uuid::parse_str(&self.worktree_id).ok()?),
            task_id: self
                .task_id
                .map(|value| uuid::Uuid::parse_str(&value))
                .transpose()
                .ok()?,
            harness: match self.harness.as_str() {
                "claude-code" => Harness::ClaudeCode,
                "codex" => Harness::Codex,
                "hermes" => Harness::Hermes,
                other => Harness::Other(other.to_owned()),
            },
            native_session_id: self.native_session_id,
            event_type: EventType::from_name(&self.event_type)?,
            occurred_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(
                self.occurred_at_ns,
            ))
            .ok()?,
            observed_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(
                self.observed_at_ns,
            ))
            .ok()?,
            source_locator: self.source_locator,
            source_offset: self.source_offset,
            git_head: self.git_head,
            git_branch: self.git_branch,
            payload: serde_json::from_str(&self.payload_json).ok()?,
            raw: serde_json::from_str(&self.raw_json).ok()?,
        })
    }
}

fn insert_quarantine(
    transaction: &rusqlite::Transaction<'_>,
    source_id: &str,
    record: &QuarantinedRecord,
) -> Result<usize> {
    Ok(transaction.execute(
        r#"
        INSERT INTO quarantine(
            source_id, source_locator, source_offset, raw_hash, error, observed_at_ns
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            source_id,
            record.source_locator,
            record.source_offset,
            record.raw_hash.as_slice(),
            record.error,
            timestamp_ns(record.observed_at)?,
        ],
    )?)
}

fn insert_capture_gap(
    transaction: &rusqlite::Transaction<'_>,
    source_id: &str,
    gap: &CaptureGapRecord,
) -> Result<usize> {
    Ok(transaction.execute(
        r#"
        INSERT INTO capture_gaps(
            source_id, expected_cursor_json, observed_cursor_json, reason, observed_at_ns
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            source_id,
            serde_json::to_string(&gap.expected_cursor)?,
            gap.observed_cursor
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            gap.reason,
            timestamp_ns(gap.observed_at)?,
        ],
    )?)
}

fn insert_event(transaction: &rusqlite::Transaction<'_>, event: &NormalizedEvent) -> Result<usize> {
    let occurred_at_ns = timestamp_ns(event.occurred_at)?;
    let observed_at_ns = timestamp_ns(event.observed_at)?;
    let payload_json = serde_json::to_string(&event.payload)?;
    let raw_json = serde_json::to_string(&event.raw)?;
    Ok(transaction.execute(
        r#"
        INSERT INTO events(
            event_id, project_id, worktree_id, task_id, harness,
            native_session_id, native_turn_id, event_type,
            occurred_at_ns, observed_at_ns, source_locator, source_offset,
            source_schema, raw_hash, idempotency_key, git_head, git_branch,
            payload_json, raw_json
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8,
            ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17,
            ?18, ?19
        )
        ON CONFLICT(idempotency_key) DO NOTHING
        "#,
        params![
            event.event_id.to_string(),
            event.project_id.0.to_string(),
            event.worktree_id.0.to_string(),
            event.task_id.map(|id| id.to_string()),
            event.harness.as_str(),
            event.native_session_id,
            event.native_turn_id,
            event.event_type.as_str(),
            occurred_at_ns,
            observed_at_ns,
            event.source_locator,
            event.source_offset,
            event.source_schema,
            event.raw_hash.as_slice(),
            event.idempotency_key.as_slice(),
            event.git_head,
            event.git_branch,
            payload_json,
            raw_json,
        ],
    )?)
}
