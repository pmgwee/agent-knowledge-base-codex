//! Retiring memories nothing ever asks for — and refusing to, until the evidence is old enough.
//!
//! Eviction is the one lifecycle operation that can destroy value silently. A retrieval bug that
//! drops recall shows up as a bad answer; an eviction policy that retires the wrong claims shows up
//! as nothing at all, because the memory that would have contradicted you is gone. So this is built
//! to refuse before it is built to act.
//!
//! **The gate is not caution, it is arithmetic.** "Never retrieved" separates a memory nothing wants
//! from a memory nothing has *had the chance* to want only after the counter has run for a while. On
//! the day counting ships, every memory in the corpus is never-retrieved — a policy reading that
//! number would retire all 2,097 of them and produce a defensible-looking reason for each. That is
//! why `memory_access_epoch` exists and why nothing here runs until
//! [`MINIMUM_OBSERVATION`] has elapsed since it.
//!
//! **Nothing is deleted.** Eviction tombstones, exactly as `brain forget` does, so the evidence and
//! the memory remain and the withdrawal is itself on the record. "Evicted" here means "no longer
//! returned", never "gone".

use anyhow::{Result, ensure};
use brain_domain::Authority;
use rusqlite::params;

use crate::CURRENT_CLAIM;
use crate::cursor::timestamp_ns;
use crate::ledger::EventLedger;

/// How long access must have been counted before eviction may run at all.
///
/// Thirty days is not a tuned number and should not be presented as one. It is roughly the point at
/// which a memory relevant to recurring work has plausibly had an occasion to be retrieved, so
/// silence starts to carry information. Shorten it and the policy begins retiring memories for the
/// crime of being newer than the instrument measuring them.
pub const MINIMUM_OBSERVATION: time::Duration = time::Duration::days(30);

/// How old a never-retrieved memory must be before it is a candidate.
///
/// Distinct from [`MINIMUM_OBSERVATION`], and both apply. The first asks whether the *counter* has
/// run long enough to be believed; this asks whether *this memory* has been available long enough
/// for its silence to mean anything. A memory recorded yesterday is unretrieved for a reason that
/// has nothing to do with its worth.
pub const QUIET_FOR: time::Duration = time::Duration::days(90);

/// Whether eviction may proceed, and if not, why not.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EvictionGate {
    /// Counting has run long enough for silence to mean something.
    Open,
    /// It has not. Carries what remains so the answer is a date rather than a refusal.
    TooYoung { days_remaining: i64 },
}

impl EvictionGate {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// A memory eviction would retire, with the reason it qualified.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EvictionCandidate {
    pub memory_id: uuid::Uuid,
    pub title: String,
    pub valid_from: time::OffsetDateTime,
    pub age_days: i64,
    pub reason: String,
}

/// What eviction would do. Computing this never changes anything.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EvictionPlan {
    /// How long access counting has been running, in days.
    pub observed_days: i64,
    pub gate: EvictionGate,
    pub candidates: Vec<EvictionCandidate>,
    /// Memories that met the numeric test but are held back by authority.
    pub protected: usize,
    /// Current, un-tombstoned memories in the project — the denominator for the share below.
    pub total_current: usize,
}

impl EvictionPlan {
    /// The share of the corpus this would retire, as a fraction.
    ///
    /// Worth looking at before applying. A policy proposing to retire most of a brain is describing
    /// a brain that is not being retrieved from, which is a retrieval problem wearing an eviction
    /// problem's clothes — and deleting the evidence would remove the only way to notice.
    pub fn share(&self) -> f64 {
        if self.total_current == 0 {
            return 0.0;
        }
        self.candidates.len() as f64 / self.total_current as f64
    }
}

impl EventLedger {
    /// When access counting became reliable for this ledger.
    pub fn access_epoch(&self) -> Result<time::OffsetDateTime> {
        let seconds: i64 = self.connection.query_row(
            "SELECT started_at_s FROM memory_access_epoch WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(time::OffsetDateTime::from_unix_timestamp(seconds)?)
    }

    /// What eviction would retire, without retiring it.
    ///
    /// Always safe to call, including before the gate opens — a closed gate returns the candidates
    /// it *would* have taken alongside the refusal, because the useful question on day one is "what
    /// is this going to do in a month", and a policy that will not show you that is a policy nobody
    /// can review.
    pub fn plan_eviction(&self, now: time::OffsetDateTime) -> Result<EvictionPlan> {
        let epoch = self.access_epoch()?;
        let observed = now - epoch;
        let gate = if observed >= MINIMUM_OBSERVATION {
            EvictionGate::Open
        } else {
            EvictionGate::TooYoung {
                days_remaining: (MINIMUM_OBSERVATION - observed).whole_days().max(1),
            }
        };

        let cutoff = timestamp_ns(now - QUIET_FOR)?;
        let mut statement = self.connection.prepare(&format!(
            r#"
            SELECT v.memory_id, v.title, v.valid_from_ns, v.authority
            FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            WHERE r.project_id = ?1 AND {CURRENT_CLAIM}
              AND v.valid_from_ns < ?2
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
              AND NOT EXISTS (
                  SELECT 1 FROM memory_access a WHERE a.memory_id = v.memory_id
              )
            ORDER BY v.valid_from_ns ASC
            "#
        ))?;
        let rows =
            statement.query_map(params![self.project_scope.0.to_string(), cutoff], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;

        let mut candidates = Vec::new();
        let mut protected = 0_usize;
        for row in rows {
            let (id, title, valid_from_ns, authority) = row?;
            // A human said this. Disuse is not a verdict on a claim someone filed deliberately, and
            // silently retiring one would make `brain remember` a worse promise than it looks.
            if authority == Authority::HumanCorrection.as_str() {
                protected += 1;
                continue;
            }
            let valid_from =
                time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(valid_from_ns))?;
            let age_days = (now - valid_from).whole_days();
            candidates.push(EvictionCandidate {
                memory_id: uuid::Uuid::parse_str(&id)?,
                title,
                valid_from,
                age_days,
                reason: format!("never retrieved in {age_days} days"),
            });
        }

        let total_current: i64 = self.connection.query_row(
            &format!(
                r#"
            SELECT COUNT(*) FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            WHERE r.project_id = ?1 AND {CURRENT_CLAIM}
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
            "#
            ),
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;

        Ok(EvictionPlan {
            observed_days: observed.whole_days().max(0),
            gate,
            candidates,
            protected,
            total_current: usize::try_from(total_current).unwrap_or(0),
        })
    }

    /// Apply a plan, tombstoning each candidate. Returns how many were retired.
    ///
    /// Re-plans rather than trusting the plan it was handed: a plan is a snapshot, and between
    /// showing it to someone and their agreeing to it, retrieval may well have reached one of these
    /// memories — which is precisely the signal that it should not be retired. Acting on the stale
    /// list would delete the thing that just proved itself useful.
    pub fn apply_eviction(&mut self, now: time::OffsetDateTime, evicted_by: &str) -> Result<usize> {
        let plan = self.plan_eviction(now)?;
        match plan.gate {
            EvictionGate::Open => {}
            EvictionGate::TooYoung { days_remaining } => {
                ensure!(
                    false,
                    "access counting has run for {} of the {} days eviction needs; {days_remaining} \
                     to go. Until then \"never retrieved\" cannot tell a memory nothing wants from \
                     one nothing has had the chance to want",
                    plan.observed_days,
                    MINIMUM_OBSERVATION.whole_days()
                );
            }
        }
        let mut retired = 0_usize;
        for candidate in &plan.candidates {
            self.forget_memory(candidate.memory_id, &candidate.reason, evicted_by, now)?;
            retired += 1;
        }
        Ok(retired)
    }
}
