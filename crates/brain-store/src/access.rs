//! What retrieval has actually reached.
//!
//! Decay needs an input, and the only honest one is use. Age alone does not make a claim wrong — a
//! decision from March can be perfectly current — so a policy that retired memories by age would
//! be retiring correct knowledge on a schedule. What separates a memory worth keeping from one
//! worth retiring is whether anything ever asks for it, and nothing was recording that. Any
//! eviction policy built before this would have been guessing.
//!
//! **This counts; it does not judge.** That division is the whole design: a model deciding what to
//! forget leaves no evidence trail, which turns a verifiable brain into a plausible one. The model
//! proposes memories; derivation decides what happens to them, from numbers anyone can check.
//!
//! Recording is best-effort at every call site. A search that cannot write a counter has still
//! answered the question, and failing the search to protect a statistic would be the wrong trade in
//! the wrong direction.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::cursor::timestamp_ns;
use crate::ledger::EventLedger;

/// How often a memory has been retrieved, and when it last was.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct MemoryAccess {
    pub retrieved_count: u64,
    pub last_retrieved_at: time::OffsetDateTime,
}

impl EventLedger {
    /// Record that these memories were returned to a caller.
    ///
    /// Takes a slice because retrieval returns a page, not a row: counting one hit per search
    /// rather than one per result would make a memory that always appears eighth look exactly as
    /// used as one that never appears at all.
    pub fn record_memory_access(
        &self,
        memory_ids: &[uuid::Uuid],
        now: time::OffsetDateTime,
    ) -> Result<()> {
        if memory_ids.is_empty() {
            return Ok(());
        }
        let at = timestamp_ns(now)?;
        let project = self.project_scope.0.to_string();
        let mut statement = self.connection.prepare_cached(
            "INSERT INTO memory_access(memory_id, project_id, retrieved_count, last_retrieved_at_ns)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(memory_id) DO UPDATE SET
                 retrieved_count = retrieved_count + 1,
                 last_retrieved_at_ns = excluded.last_retrieved_at_ns",
        )?;
        for memory_id in memory_ids {
            statement.execute(params![memory_id.to_string(), &project, at])?;
        }
        Ok(())
    }

    pub fn memory_access(&self, memory_id: uuid::Uuid) -> Result<Option<MemoryAccess>> {
        self.connection
            .query_row(
                "SELECT retrieved_count, last_retrieved_at_ns FROM memory_access
                 WHERE memory_id = ?1 AND project_id = ?2",
                params![memory_id.to_string(), self.project_scope.0.to_string()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .map(|(count, at)| {
                Ok(MemoryAccess {
                    retrieved_count: u64::try_from(count).unwrap_or(0),
                    last_retrieved_at: time::OffsetDateTime::from_unix_timestamp_nanos(
                        i128::from(at),
                    )?,
                })
            })
            .transpose()
    }

    /// How many current memories retrieval has never reached.
    ///
    /// The number that makes decay discussable. A brain where most memories are never retrieved is
    /// storing rather than remembering, and until now there was no way to tell the two apart.
    pub fn never_retrieved_memory_count(&self) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            r#"
            SELECT COUNT(*) FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            WHERE r.project_id = ?1 AND v.status = 'current'
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
              AND NOT EXISTS (
                  SELECT 1 FROM memory_access a WHERE a.memory_id = v.memory_id
              )
            "#,
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
