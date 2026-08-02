use std::path::Path;

use anyhow::{Result, bail};
use brain_domain::{EventBatch, NormalizedEvent, ProjectId, SourceCursor};
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
        save_cursor(&transaction, &batch.source_id, &batch.next_cursor)?;
        transaction.commit()?;
        Ok(AppendResult { inserted })
    }

    pub fn event_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?)
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
