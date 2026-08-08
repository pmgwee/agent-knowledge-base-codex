//! Withdrawing a claim without deleting it.
//!
//! Evidence is append-only, which is what makes every citation in this brain checkable — and it is
//! also why there was no way to take anything back. Those two facts sat in tension for as long as
//! the ledger existed: a hard delete would break the invariant, and having no withdrawal at all
//! leaves someone unable to remove a memory of something they did not mean to capture, from a
//! system holding every keystroke of their work.
//!
//! A tombstone resolves it. The memory and its evidence stay exactly where they are; a new row says
//! the memory must no longer be returned, projected, or read. Every read path honours it, so what a
//! user sees is deletion, while the ledger only ever grew. The withdrawal becomes a fact on the
//! record instead of a silent hole — which is the difference between a system you can audit and one
//! where absence proves nothing.
//!
//! **Every read path must honour this or the guarantee is worthless.** Search, projection, and
//! export all filter on it; a path added later that forgets to would resurrect a withdrawn memory
//! on some machines and not others, which is the worst possible failure for this particular
//! feature.

use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};

use crate::cursor::timestamp_ns;
use crate::ledger::EventLedger;

/// A withdrawn memory, and the record of who withdrew it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Tombstone {
    pub memory_id: uuid::Uuid,
    pub reason: String,
    pub redacted_by: String,
    pub redacted_at: time::OffsetDateTime,
}

impl EventLedger {
    /// Withdraw a memory. Idempotent: forgetting twice is not an error.
    ///
    /// The memory must exist. Withdrawing something absent is far more likely to be a mistyped id
    /// than an intent, and silently succeeding would tell the user they had removed something when
    /// they had not.
    pub fn forget_memory(
        &mut self,
        memory_id: uuid::Uuid,
        reason: &str,
        redacted_by: &str,
        now: time::OffsetDateTime,
    ) -> Result<Tombstone> {
        ensure!(!reason.trim().is_empty(), "a withdrawal needs a reason");
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_records WHERE memory_id = ?1 AND project_id = ?2)",
            params![memory_id.to_string(), self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        ensure!(exists, "no memory {memory_id} in this project");

        self.connection
            .execute(
                "INSERT INTO memory_tombstones(
                     memory_id, project_id, reason, redacted_by, redacted_at_ns
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(memory_id) DO NOTHING",
                params![
                    memory_id.to_string(),
                    self.project_scope.0.to_string(),
                    reason.trim(),
                    redacted_by,
                    timestamp_ns(now)?,
                ],
            )
            .context("record memory tombstone")?;
        // Same reason as `append_memory`: a write on this connection does not move
        // `PRAGMA data_version`, so a cached search would keep returning what was just withdrawn.
        self.search_cache.borrow_mut().clear();

        self.tombstone(memory_id)?
            .context("tombstone vanished immediately after being written")
    }

    pub fn tombstone(&self, memory_id: uuid::Uuid) -> Result<Option<Tombstone>> {
        self.connection
            .query_row(
                "SELECT memory_id, reason, redacted_by, redacted_at_ns
                 FROM memory_tombstones WHERE memory_id = ?1 AND project_id = ?2",
                params![memory_id.to_string(), self.project_scope.0.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(memory_id, reason, redacted_by, at)| {
                Ok(Tombstone {
                    memory_id: uuid::Uuid::parse_str(&memory_id)?,
                    reason,
                    redacted_by,
                    redacted_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(at))?,
                })
            })
            .transpose()
    }

    /// Every withdrawal in this project, newest first.
    ///
    /// Exported in place of the content it withdrew, so a bundle records that something was taken
    /// back rather than quietly omitting it — an export with a gap and no explanation is the same
    /// shape as an export that lost data.
    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        let mut statement = self.connection.prepare(
            "SELECT memory_id, reason, redacted_by, redacted_at_ns
             FROM memory_tombstones WHERE project_id = ?1
             ORDER BY redacted_at_ns DESC",
        )?;
        let rows = statement.query_map([self.project_scope.0.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (memory_id, reason, redacted_by, at) = row?;
            Ok(Tombstone {
                memory_id: uuid::Uuid::parse_str(&memory_id)?,
                reason,
                redacted_by,
                redacted_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(at))?,
            })
        })
        .collect()
    }

    /// The withdrawn memory ids, for filtering a read path.
    pub(crate) fn tombstoned_ids(&self) -> Result<std::collections::HashSet<uuid::Uuid>> {
        let mut statement = self
            .connection
            .prepare("SELECT memory_id FROM memory_tombstones WHERE project_id = ?1")?;
        let rows = statement.query_map([self.project_scope.0.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| Ok(uuid::Uuid::parse_str(&row?)?)).collect()
    }
}
