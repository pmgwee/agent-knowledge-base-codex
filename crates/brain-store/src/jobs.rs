use anyhow::{Context, Result, ensure};
use brain_domain::ProjectId;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::EventLedger;
use crate::cursor::timestamp_ns;

/// Most events a single consolidation job may span.
///
/// A job is loaded whole into one evidence packet and handed to a model, so its size is an API
/// request, not an internal detail. Registration ingests a project's entire transcript history
/// in one burst — tens of thousands of events — and without a bound the first job tries to
/// carry all of it.
pub const MAX_JOB_EVENTS: usize = 200;

/// Most evidence bytes a single consolidation job may span, counting `payload` *and* `raw`.
///
/// Both are counted because both are sent: `RedactedEvidence` carries each event's payload and
/// its raw form, so a bound on payload alone understates the request. On the first job measured
/// against the live ledger that gap was 2.6x — 277 KB bounded, 730 KB actually sent.
///
/// Event sizes vary by three orders of magnitude here: a `tool.requested` runs ~2 KB while a
/// `session.compacted` has been measured at 1.3 MB, and one captured event reached 3.7 MB. A
/// count-only bound would therefore still admit wildly different packets, so cost is bounded by
/// bytes as well and whichever limit is reached first ends the window.
///
/// 400 KB is roughly 100k tokens. Measured against the provider directly, a 730 KB body with
/// `response_format: json_object` answered in 9 s, so this leaves real headroom rather than
/// sitting at the edge of what works.
pub const MAX_JOB_PAYLOAD_BYTES: usize = 400_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsolidationReason {
    SessionStopped,
    SessionCompacted,
    ExplicitCheckpoint,
    Inactivity,
    EventThreshold,
}

impl ConsolidationReason {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStopped => "session_stopped",
            Self::SessionCompacted => "session_compacted",
            Self::ExplicitCheckpoint => "explicit_checkpoint",
            Self::Inactivity => "inactivity",
            Self::EventThreshold => "event_threshold",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "session_stopped" => Self::SessionStopped,
            "session_compacted" => Self::SessionCompacted,
            "explicit_checkpoint" => Self::ExplicitCheckpoint,
            "inactivity" => Self::Inactivity,
            "event_threshold" => Self::EventThreshold,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Pending,
    Leased,
    Completed,
    DeadLetter,
}

impl JobStatus {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Completed => "completed",
            Self::DeadLetter => "dead_letter",
        }
    }

    fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "leased" => Self::Leased,
            "completed" => Self::Completed,
            "dead_letter" => Self::DeadLetter,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ConsolidationJob {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub first_event_id: uuid::Uuid,
    pub last_event_id: uuid::Uuid,
    pub reason: ConsolidationReason,
    pub status: JobStatus,
    pub attempt: u32,
    pub available_at: time::OffsetDateTime,
    pub lease_owner: Option<String>,
    pub lease_until: Option<time::OffsetDateTime>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionManifestEntry {
    pub category: String,
    pub token_hash: [u8; 32],
}

impl EventLedger {
    pub fn enqueue_consolidation_job(
        &mut self,
        first_event_id: uuid::Uuid,
        last_event_id: uuid::Uuid,
        reason: ConsolidationReason,
    ) -> Result<ConsolidationJob> {
        for event_id in [first_event_id, last_event_id] {
            let exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?1 AND project_id = ?2)",
                params![event_id.to_string(), self.project_scope.0.to_string()],
                |row| row.get(0),
            )?;
            ensure!(exists, "job event {event_id} is outside the project ledger");
        }
        let idempotency_key = job_key(self.project_scope, first_event_id, last_event_id, &reason);
        let job_id = deterministic_uuid(idempotency_key);
        let now = time::OffsetDateTime::now_utc();
        self.connection.execute(
            r#"
            INSERT INTO consolidation_jobs(
                job_id, project_id, first_event_id, last_event_id, reason,
                idempotency_key, status, attempt, available_at_ns, created_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7)
            ON CONFLICT(idempotency_key) DO NOTHING
            "#,
            params![
                job_id.to_string(),
                self.project_scope.0.to_string(),
                first_event_id.to_string(),
                last_event_id.to_string(),
                reason.as_str(),
                idempotency_key.as_slice(),
                timestamp_ns(now)?,
            ],
        )?;
        self.consolidation_job(job_id)?
            .context("enqueued consolidation job is missing")
    }

    pub fn consolidation_job(&self, job_id: uuid::Uuid) -> Result<Option<ConsolidationJob>> {
        self.connection
            .query_row(
                r#"
                SELECT job_id, project_id, first_event_id, last_event_id, reason,
                       status, attempt, available_at_ns, lease_owner, lease_until_ns,
                       last_error
                FROM consolidation_jobs
                WHERE job_id = ?1 AND project_id = ?2
                "#,
                params![job_id.to_string(), self.project_scope.0.to_string()],
                parse_job,
            )
            .optional()?
            .map(parse_job_record)
            .transpose()
    }

    pub fn consolidation_job_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM consolidation_jobs WHERE project_id = ?1",
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?)
    }

    pub fn enqueue_event_threshold_job(
        &mut self,
        threshold: usize,
    ) -> Result<Option<ConsolidationJob>> {
        ensure!(threshold > 0, "event threshold must be positive");
        let Some((first, last, count, _)) = self.unqueued_event_range()? else {
            return Ok(None);
        };
        if count < u64::try_from(threshold)? {
            return Ok(None);
        }
        self.enqueue_consolidation_job(first, last, ConsolidationReason::EventThreshold)
            .map(Some)
    }

    pub fn enqueue_through_event_job(
        &mut self,
        last_event_id: uuid::Uuid,
        reason: ConsolidationReason,
    ) -> Result<Option<ConsolidationJob>> {
        let last_row: i64 = self.connection.query_row(
            "SELECT rowid FROM events WHERE event_id = ?1 AND project_id = ?2",
            params![last_event_id.to_string(), self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        let last_covered_row: i64 = self.connection.query_row(
            r#"
            SELECT COALESCE(MAX(e.rowid), 0)
            FROM consolidation_jobs j
            JOIN events e ON e.event_id = j.last_event_id
            WHERE j.project_id = ?1
            "#,
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        if last_covered_row >= last_row {
            return Ok(None);
        }
        let first: String = self.connection.query_row(
            r#"
            SELECT event_id FROM events
            WHERE project_id = ?1 AND rowid > ?2 AND rowid <= ?3
            ORDER BY rowid ASC LIMIT 1
            "#,
            params![self.project_scope.0.to_string(), last_covered_row, last_row,],
            |row| row.get(0),
        )?;
        self.enqueue_consolidation_job(uuid::Uuid::parse_str(&first)?, last_event_id, reason)
            .map(Some)
    }

    pub fn enqueue_inactivity_job(
        &mut self,
        now: time::OffsetDateTime,
        inactivity: time::Duration,
    ) -> Result<Option<ConsolidationJob>> {
        ensure!(
            inactivity.is_positive(),
            "inactivity duration must be positive"
        );
        let Some((first, last, _, latest_observed)) = self.unqueued_event_range()? else {
            return Ok(None);
        };
        if latest_observed > now - inactivity {
            return Ok(None);
        }
        self.enqueue_consolidation_job(first, last, ConsolidationReason::Inactivity)
            .map(Some)
    }

    pub fn lease_consolidation_job(
        &mut self,
        worker: &str,
        now: time::OffsetDateTime,
        lease_duration: time::Duration,
    ) -> Result<Option<ConsolidationJob>> {
        ensure!(!worker.trim().is_empty(), "worker identity is empty");
        ensure!(
            lease_duration.is_positive(),
            "lease duration must be positive"
        );
        let now_ns = timestamp_ns(now)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job_id: Option<String> = transaction
            .query_row(
                r#"
                SELECT job_id
                FROM consolidation_jobs
                WHERE project_id = ?1
                  AND (
                    (status = 'pending' AND available_at_ns <= ?2)
                    OR (status = 'leased' AND lease_until_ns <= ?2)
                  )
                ORDER BY available_at_ns ASC, created_at_ns ASC, job_id ASC
                LIMIT 1
                "#,
                params![self.project_scope.0.to_string(), now_ns],
                |row| row.get(0),
            )
            .optional()?;
        let Some(job_id) = job_id else {
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute(
            r#"
            UPDATE consolidation_jobs SET
                status = 'leased',
                attempt = attempt + 1,
                lease_owner = ?2,
                lease_until_ns = ?3,
                last_error = NULL
            WHERE job_id = ?1
            "#,
            params![job_id, worker, timestamp_ns(now + lease_duration)?,],
        )?;
        let raw = transaction.query_row(
            r#"
            SELECT job_id, project_id, first_event_id, last_event_id, reason,
                   status, attempt, available_at_ns, lease_owner, lease_until_ns,
                   last_error
            FROM consolidation_jobs WHERE job_id = ?1
            "#,
            [job_id],
            parse_job,
        )?;
        transaction.commit()?;
        Ok(Some(parse_job_record(raw)?))
    }

    pub fn complete_consolidation_job(
        &mut self,
        job_id: uuid::Uuid,
        worker: &str,
        now: time::OffsetDateTime,
    ) -> Result<()> {
        let updated = self.connection.execute(
            r#"
            UPDATE consolidation_jobs SET
                status = 'completed', completed_at_ns = ?3,
                lease_owner = NULL, lease_until_ns = NULL, last_error = NULL
            WHERE job_id = ?1 AND lease_owner = ?2 AND status = 'leased'
            "#,
            params![job_id.to_string(), worker, timestamp_ns(now)?],
        )?;
        ensure!(
            updated == 1,
            "consolidation job lease is not owned by {worker}"
        );
        Ok(())
    }

    pub fn fail_consolidation_job(
        &mut self,
        job_id: uuid::Uuid,
        worker: &str,
        error: &str,
        now: time::OffsetDateTime,
    ) -> Result<()> {
        let job = self
            .consolidation_job(job_id)?
            .context("consolidation job does not exist")?;
        ensure!(
            job.status == JobStatus::Leased && job.lease_owner.as_deref() == Some(worker),
            "consolidation job lease is not owned by {worker}"
        );
        let (status, available_at) = if job.attempt >= 5 {
            (JobStatus::DeadLetter, now)
        } else {
            let delay_seconds = 1_i64 << job.attempt;
            (
                JobStatus::Pending,
                now + time::Duration::seconds(delay_seconds),
            )
        };
        self.connection.execute(
            r#"
            UPDATE consolidation_jobs SET
                status = ?3, available_at_ns = ?4,
                lease_owner = NULL, lease_until_ns = NULL, last_error = ?5
            WHERE job_id = ?1 AND lease_owner = ?2
            "#,
            params![
                job_id.to_string(),
                worker,
                status.as_str(),
                timestamp_ns(available_at)?,
                bounded_error(error),
            ],
        )?;
        Ok(())
    }

    pub fn record_redaction_manifest(
        &mut self,
        job_id: uuid::Uuid,
        entries: &[RedactionManifestEntry],
    ) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for entry in entries {
            transaction.execute(
                "INSERT OR IGNORE INTO redaction_manifests(job_id, category, token_hash) VALUES (?1, ?2, ?3)",
                params![job_id.to_string(), entry.category, entry.token_hash.as_slice()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn redaction_manifest(&self, job_id: uuid::Uuid) -> Result<Vec<RedactionManifestEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT category, token_hash FROM redaction_manifests WHERE job_id = ?1 ORDER BY category, token_hash",
        )?;
        let rows = statement.query_map([job_id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (category, hash) = row?;
            Ok(RedactionManifestEntry {
                category,
                token_hash: hash
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("redaction hash is not 32 bytes"))?,
            })
        })
        .collect()
    }

    fn unqueued_event_range(
        &self,
    ) -> Result<Option<(uuid::Uuid, uuid::Uuid, u64, time::OffsetDateTime)>> {
        let last_covered_row: i64 = self.connection.query_row(
            r#"
            SELECT COALESCE(MAX(e.rowid), 0)
            FROM consolidation_jobs j
            JOIN events e ON e.event_id = j.last_event_id
            WHERE j.project_id = ?1
            "#,
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        // Find where this window has to stop. Every event after `last_covered_row` is
        // uncovered, but a job is eventually loaded whole into one evidence packet and sent to
        // a model, so the range must be bounded here — at the only place that knows how far it
        // is about to reach. Without this, ingesting a project's history in one burst enqueues
        // a single job spanning the entire backlog, which no context window can hold.
        let last_row_in_window: Option<i64> = self
            .connection
            .query_row(
                r#"
                SELECT MAX(rowid) FROM (
                    SELECT rowid,
                           SUM(bytes) OVER (ORDER BY rowid) AS running_bytes,
                           ROW_NUMBER() OVER (ORDER BY rowid) AS position
                    FROM (
                        -- Bound the candidate set *before* the running sum. Without this LIMIT
                        -- the window function reads every uncovered event on every call — on a
                        -- freshly registered project that is tens of thousands of rows and
                        -- hundreds of megabytes of payload text, recomputed each tick. That
                        -- saturates a core and starves the runtime the provider call runs on,
                        -- so consolidation appears to hang on a healthy connection.
                        -- Both columns, because the evidence packet carries `payload` *and*
                        -- `raw` for every event. Counting payload alone understated the real
                        -- request body by 2.6x on the first measured job: 277 KB bounded,
                        -- 730 KB actually sent.
                        SELECT rowid,
                               LENGTH(COALESCE(payload_json, ''))
                             + LENGTH(COALESCE(raw_json, '')) AS bytes
                        FROM events
                        WHERE project_id = ?1 AND rowid > ?2
                        ORDER BY rowid
                        LIMIT ?3
                    )
                )
                WHERE position = 1 OR running_bytes <= ?4
                "#,
                params![
                    self.project_scope.0.to_string(),
                    last_covered_row,
                    i64::try_from(MAX_JOB_EVENTS)?,
                    i64::try_from(MAX_JOB_PAYLOAD_BYTES)?,
                ],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(last_row_in_window) = last_row_in_window else {
            return Ok(None);
        };
        let range: Option<(String, String, i64, i64)> = self
            .connection
            .query_row(
                r#"
                SELECT
                    (SELECT event_id FROM events
                     WHERE project_id = ?1 AND rowid > ?2 AND rowid <= ?3
                     ORDER BY rowid ASC LIMIT 1),
                    (SELECT event_id FROM events
                     WHERE project_id = ?1 AND rowid > ?2 AND rowid <= ?3
                     ORDER BY rowid DESC LIMIT 1),
                    COUNT(*),
                    MAX(observed_at_ns)
                FROM events
                WHERE project_id = ?1 AND rowid > ?2 AND rowid <= ?3
                HAVING COUNT(*) > 0
                "#,
                params![
                    self.project_scope.0.to_string(),
                    last_covered_row,
                    last_row_in_window
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        range
            .map(|(first, last, count, observed_at)| {
                Ok((
                    uuid::Uuid::parse_str(&first)?,
                    uuid::Uuid::parse_str(&last)?,
                    u64::try_from(count)?,
                    from_ns(observed_at)?,
                ))
            })
            .transpose()
    }
}

type RawJob = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    Option<i64>,
    Option<String>,
);

fn parse_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawJob> {
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
        row.get(9)?,
        row.get(10)?,
    ))
}

fn parse_job_record(raw: RawJob) -> Result<ConsolidationJob> {
    Ok(ConsolidationJob {
        id: uuid::Uuid::parse_str(&raw.0)?,
        project_id: ProjectId(uuid::Uuid::parse_str(&raw.1)?),
        first_event_id: uuid::Uuid::parse_str(&raw.2)?,
        last_event_id: uuid::Uuid::parse_str(&raw.3)?,
        reason: ConsolidationReason::from_name(&raw.4).context("invalid job reason")?,
        status: JobStatus::from_name(&raw.5).context("invalid job status")?,
        attempt: u32::try_from(raw.6)?,
        available_at: from_ns(raw.7)?,
        lease_owner: raw.8,
        lease_until: raw.9.map(from_ns).transpose()?,
        last_error: raw.10,
    })
}

fn job_key(
    project: ProjectId,
    first: uuid::Uuid,
    last: uuid::Uuid,
    reason: &ConsolidationReason,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(project.0.as_bytes());
    hasher.update(first.as_bytes());
    hasher.update(last.as_bytes());
    hasher.update(reason.as_str().as_bytes());
    hasher.finalize().into()
}

fn deterministic_uuid(hash: [u8; 32]) -> uuid::Uuid {
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes)
}

fn bounded_error(error: &str) -> String {
    error.chars().take(2_000).collect()
}

fn from_ns(value: i64) -> Result<time::OffsetDateTime> {
    Ok(time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(value),
    )?)
}
