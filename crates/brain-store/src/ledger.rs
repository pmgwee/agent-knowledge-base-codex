use std::path::Path;

use anyhow::{Result, bail};
use brain_domain::{
    CaptureGapRecord, EventBatch, NormalizedEvent, ProjectId, QuarantinedRecord, SourceCursor,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::cursor::{load_cursor, save_cursor, timestamp_ns};
use crate::migrations::{configure, migrate};

pub struct EventLedger {
    connection: Connection,
    project_scope: ProjectId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendResult {
    pub inserted: usize,
    pub quarantined: usize,
    pub capture_gaps: usize,
}

impl EventLedger {
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
