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

    /// Memories nothing has retrieved and whose evidence has gone quiet.
    ///
    /// Both conditions, not either. Age alone is not staleness — a decision from March can be
    /// perfectly current — and disuse alone is not either, since a memory recorded yesterday has
    /// had no chance to be used. A claim is stale when it is *old* and *nothing ever wanted it*,
    /// which is the weakest statement that carries any information.
    ///
    /// Reversible by construction: the moment retrieval returns one, it has an access row and
    /// stops being stale. Nothing needs to clear a flag, because there is no flag — staleness is
    /// computed from two facts that both move on their own.
    pub fn stale_memory_ids(
        &self,
        now: time::OffsetDateTime,
        quiet_for: time::Duration,
    ) -> Result<std::collections::HashSet<uuid::Uuid>> {
        let cutoff = timestamp_ns(now - quiet_for)?;
        let mut statement = self.connection.prepare(
            r#"
            SELECT v.memory_id
            FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            WHERE r.project_id = ?1 AND v.status = 'current'
              AND v.valid_from_ns < ?2
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
              AND NOT EXISTS (
                  SELECT 1 FROM memory_access a WHERE a.memory_id = v.memory_id
              )
            "#,
        )?;
        let rows = statement
            .query_map(params![self.project_scope.0.to_string(), cutoff], |row| {
                row.get::<_, String>(0)
            })?;
        rows.map(|row| Ok(uuid::Uuid::parse_str(&row?)?)).collect()
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

/// How long an unused memory takes to lose half its retention, before strengthening.
///
/// Ninety days matches the staleness threshold the projection already uses, so the continuous score
/// and the boolean flag disagree as little as possible. It is a starting point from the same place
/// that one was — not a tuned constant, and the benchmark is what would move it.
pub const RETENTION_HALF_LIFE_DAYS: f64 = 90.0;

/// Retention below which a memory is considered stale.
///
/// `0.5` is one half-life for a never-retrieved memory, which is exactly the old boolean rule. The
/// point of the score is not to change that line but to say *how far* a memory sits from it, and to
/// let use move it.
pub const STALE_RETENTION: f64 = 0.5;

/// A memory's derived retention, and the inputs that produced it.
///
/// Every field is a fact from the ledger. Nothing here is a model's opinion, which is the whole
/// reason decay is allowed to be automatic at all.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct MemoryRetention {
    pub memory_id: uuid::Uuid,
    /// Ebbinghaus retention in `(0, 1]`. Higher means better retained.
    pub retention: f64,
    /// How many times retrieval has returned it.
    pub retrieved_count: u64,
    /// Days since it was last returned, or since it was recorded if never.
    pub quiet_days: f64,
    /// The multiplier retrieval has earned it.
    pub strength: f64,
}

impl MemoryRetention {
    pub fn is_stale(&self) -> bool {
        self.retention < STALE_RETENTION
    }
}

/// Ebbinghaus retention: `exp(-t · ln2 / (half_life · strength))`.
///
/// The `ln 2` is not decoration. Without it the constant is a *time constant*, not a half-life:
/// `exp(-1)` is 0.368, so an unused memory would cross the 0.5 stale line at 62 days while the
/// constant claimed 90. A name that means something different from the arithmetic beside it is the
/// quiet kind of wrong — it reads correctly forever.
///
/// **Strengthening is the half that makes this more than an age check.** A memory retrieved ten
/// times decays roughly `1 + ln(11) ≈ 3.4` times slower than one never retrieved, so use buys
/// survival rather than merely resetting a clock. `ln` rather than a linear term deliberately: the
/// tenth retrieval should matter less than the first, or a single hot memory would become
/// permanent and crowd out everything the corpus learned since.
///
/// Pure, so it is testable without a ledger and auditable without running one.
pub fn retention_score(retrieved_count: u64, quiet_days: f64) -> (f64, f64) {
    let strength = 1.0 + (1.0 + retrieved_count as f64).ln();
    let quiet = quiet_days.max(0.0);
    let retention = (-quiet * std::f64::consts::LN_2 / (RETENTION_HALF_LIFE_DAYS * strength)).exp();
    (retention, strength)
}

impl EventLedger {
    /// Retention for every current memory, newest-quietest first.
    ///
    /// Replaces nothing: `stale_memory_ids` still answers the boolean question the projection asks.
    /// This answers *how stale*, which is what ranking needs and a flag cannot express.
    pub fn memory_retention(&self, now: time::OffsetDateTime) -> Result<Vec<MemoryRetention>> {
        let now_ns = timestamp_ns(now)?;
        let mut statement = self.connection.prepare(
            r#"
            SELECT v.memory_id,
                   coalesce(a.retrieved_count, 0),
                   coalesce(a.last_retrieved_at_ns, v.valid_from_ns)
            FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            LEFT JOIN memory_access a ON a.memory_id = v.memory_id
            WHERE r.project_id = ?1 AND v.status = 'current'
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
            "#,
        )?;
        let rows = statement.query_map(params![self.project_scope.0.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (id, count, last_ns) = row?;
            let retrieved_count = u64::try_from(count).unwrap_or(0);
            let quiet_days = (now_ns - last_ns).max(0) as f64 / 86_400_000_000_000.0;
            let (retention, strength) = retention_score(retrieved_count, quiet_days);
            out.push(MemoryRetention {
                memory_id: uuid::Uuid::parse_str(&id)?,
                retention,
                retrieved_count,
                quiet_days,
                strength,
            });
        }
        out.sort_by(|a, b| b.retention.total_cmp(&a.retention));
        Ok(out)
    }
}
