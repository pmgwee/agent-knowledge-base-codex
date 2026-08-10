use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::Result;
use brain_context::{ConsolidationLlm, EvidencePacket, RedactedEvidence, validate_proposed_batch};
use brain_store::{ConsolidationJob, EventLedger, JobStatus, RedactionManifestEntry, StoredEvent};
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::{ConsolidationProviderConfig, ServiceLaunchConfig};

/// How many provider calls may be in flight across all projects at once.
///
/// Consolidation used to await every call in series — `for project in &config.projects` awaited
/// each project, and the inner loop awaited each job — so three projects and eight slots produced
/// no concurrency whatever. At ~40 s per call that capped the whole service near 90 jobs/hour,
/// which is the ceiling every measurement kept landing under: 2,020 jobs on the best day observed,
/// and 1,955 still queued a week later.
///
/// Three is the project count, not a tuning result. It exists as a cap rather than as
/// "spawn one per project" so registering a fourth project widens the backlog rather than the
/// request rate — a quota this shares with `claude -p` is not something to discover the edge of by
/// accident.
const MAX_CONCURRENT_CONSOLIDATION: usize = 3;

/// How many jobs one project may drain in a single tick.
///
/// Deliberately still sequential *within* a project. The ledgers are separate SQLite files, so
/// concurrency across them is free; concurrency inside one would put two writers on one database
/// and two leases on one queue for no gain the provider latency would let us keep.
const JOBS_PER_PROJECT_PER_TICK: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsolidationCrashPoint {
    None,
    BeforeJobAck,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerOutcome {
    Idle,
    Completed(uuid::Uuid),
    RetryScheduled(uuid::Uuid),
    DeadLetter(uuid::Uuid),
    SimulatedCrash(uuid::Uuid),
    /// The provider was unavailable — rate limited, timed out, unreachable.
    ///
    /// Deliberately not a failure. The job is untouched: its lease is left to expire so it
    /// returns to the queue without consuming an attempt.
    ProviderUnavailable(uuid::Uuid),
}

/// Whether an error means "this provider cannot answer right now" rather than "this job is bad".
///
/// The distinction decides whether a job survives an outage. Attempts back off as
/// `1 << attempt` seconds — 2, 4, 8, 16, 32 — so five of them are spent in about a minute, and
/// a job dead-letters permanently. A rate limit lasting hours would therefore destroy every
/// job attempted during it, none of which was ever the problem.
///
/// Matching on message text is unlovely, but the alternative is threading a typed error through
/// a trait that any provider may implement, and a provider that words its outage differently
/// simply falls back to the old behaviour rather than misclassifying a real defect as transient.
fn provider_unavailable(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}").to_lowercase();
    [
        "429",
        "rate limit",
        "too many requests",
        "quota",
        "timed out",
        "timeout",
        "connect",
        "dns",
        "502",
        "503",
        "504",
        "temporarily unavailable",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

pub struct ConsolidationWorker {
    worker_id: String,
    lease_duration: time::Duration,
}

impl ConsolidationWorker {
    pub fn new(worker_id: impl Into<String>, lease_duration: time::Duration) -> Self {
        Self {
            worker_id: worker_id.into(),
            lease_duration,
        }
    }

    pub async fn run_once(
        &self,
        ledger: &mut EventLedger,
        proposer: &dyn ConsolidationLlm,
        now: time::OffsetDateTime,
        crash_point: ConsolidationCrashPoint,
    ) -> Result<WorkerOutcome> {
        let Some(job) =
            ledger.lease_consolidation_job(&self.worker_id, now, self.lease_duration)?
        else {
            return Ok(WorkerOutcome::Idle);
        };
        let events = ledger.events_between(job.first_event_id, job.last_event_id)?;
        let packet = packet_from_events(&job, events);
        ledger.record_redaction_manifest(job.id, &packet.redactions)?;
        let validated = match proposer.propose(&packet).await {
            Ok(proposed) => match validate_proposed_batch(&packet, proposed) {
                Ok(validated) => validated,
                Err(error) => return self.fail(ledger, &job, &format!("{error:#}"), now),
            },
            Err(error) if provider_unavailable(&error) => {
                // Leave the job exactly as it was. Its lease expires on its own, returning it
                // to the queue with its attempt count intact, so an outage costs time rather
                // than evidence.
                tracing::warn!(
                    job = %job.id,
                    attempt = job.attempt,
                    %error,
                    "provider unavailable; job deferred without consuming an attempt"
                );
                return Ok(WorkerOutcome::ProviderUnavailable(job.id));
            }
            Err(error) => return self.fail(ledger, &job, &format!("{error:#}"), now),
        };
        if !validated.rejected.is_empty() {
            // Surfaced, not retried. The request is made at `temperature: 0`, so a retry
            // reproduces the same rejected proposal and spends another call to fail identically.
            // What is worth knowing is which memories were dropped and why.
            tracing::warn!(
                job = %job.id,
                accepted = validated.accepted.len(),
                rejected = validated.rejected.len(),
                reasons = ?validated.rejected,
                "provider proposals rejected during validation"
            );
        }
        let proposed = validated.accepted;
        for memory in &proposed {
            if let Err(error) = ledger.append_memory(memory) {
                return self.fail(ledger, &job, &format!("{error:#}"), now);
            }
        }
        if crash_point == ConsolidationCrashPoint::BeforeJobAck {
            return Ok(WorkerOutcome::SimulatedCrash(job.id));
        }
        ledger.complete_consolidation_job(job.id, &self.worker_id, now)?;
        Ok(WorkerOutcome::Completed(job.id))
    }

    fn fail(
        &self,
        ledger: &mut EventLedger,
        job: &ConsolidationJob,
        error: &str,
        now: time::OffsetDateTime,
    ) -> Result<WorkerOutcome> {
        ledger.fail_consolidation_job(job.id, &self.worker_id, error, now)?;
        let status = ledger
            .consolidation_job(job.id)?
            .expect("leased consolidation job remains present")
            .status;
        Ok(if status == JobStatus::DeadLetter {
            WorkerOutcome::DeadLetter(job.id)
        } else {
            WorkerOutcome::RetryScheduled(job.id)
        })
    }
}

pub async fn run_configured_consolidation(
    config: ServiceLaunchConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let (_, pressure) = tokio::sync::watch::channel(crate::DegradationState::default());
    run_configured_consolidation_with_pressure(config, shutdown, pressure).await
}

pub async fn run_configured_consolidation_with_pressure(
    config: ServiceLaunchConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    pressure: tokio::sync::watch::Receiver<crate::DegradationState>,
) -> Result<()> {
    let Some(provider) = config.consolidation.clone() else {
        while shutdown.changed().await.is_ok() {
            if *shutdown.borrow() {
                break;
            }
        }
        return Ok(());
    };
    let llm: std::sync::Arc<dyn ConsolidationLlm> = match provider {
        ConsolidationProviderConfig::Glm {
            endpoint,
            model,
            api_key_env,
            timeout_ms,
            max_retries,
        } => std::sync::Arc::new(brain_context::GlmClient::new(brain_context::GlmConfig {
            endpoint,
            model,
            api_key_env,
            timeout: std::time::Duration::from_millis(timeout_ms),
            max_retries,
        })?),
    };
    let worker = std::sync::Arc::new(ConsolidationWorker::new(
        format!("service-{}", std::process::id()),
        time::Duration::minutes(2),
    ));
    // Held across ticks, not rebuilt per tick: the cap is on calls in flight, and a per-tick
    // semaphore would reset it every two seconds.
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONSOLIDATION));
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                if pressure.borrow().consolidation_paused {
                    continue;
                }
                drain_all_projects(&config.projects, &worker, &llm, &permits).await;
            }
        }
    }
}

/// Drain every project for one tick — concurrently across ledgers, one call per ledger.
///
/// One task per project, capped at [`MAX_CONCURRENT_CONSOLIDATION`] in flight, and all of them
/// awaited before this returns. **Awaiting here is what keeps the one-call-per-ledger promise**:
/// without it a slow project would still be draining when the next tick spawned a second task
/// against the same database, which is the one form of concurrency this design rules out.
async fn drain_all_projects(
    projects: &[crate::ServiceProjectConfig],
    worker: &std::sync::Arc<ConsolidationWorker>,
    llm: &std::sync::Arc<dyn ConsolidationLlm>,
    permits: &std::sync::Arc<tokio::sync::Semaphore>,
) {
    let mut drains = tokio::task::JoinSet::new();
    for project in projects {
        let ledger_path = project.ledger_path.clone();
        let project_id = project.project_id;
        let worker = std::sync::Arc::clone(worker);
        let llm = std::sync::Arc::clone(llm);
        let permits = std::sync::Arc::clone(permits);
        drains.spawn(async move {
            let Ok(_permit) = permits.acquire().await else {
                return;
            };
            // Every failure below is contained to this project and this tick.
            //
            // The whole service runs under one `try_join!`, so an error escaping here does not
            // degrade consolidation — it takes down capture, the hook pipe, rediscovery and
            // projections with it, and the process exits 1 having written nothing about why. That
            // is the "service died again" this brain has hit repeatedly: a transient SQLite lock
            // or a single unusable job, ending the run.
            //
            // Consolidation is the most failure-prone loop in the service — it is the only one
            // that depends on a network call to a third party — and it is also the least urgent.
            // It has no business deciding whether capture keeps running.
            if let Err(error) = drain_project(&ledger_path, project_id, &worker, llm.as_ref()).await
            {
                tracing::warn!(
                    project_id = %project_id.0,
                    %error,
                    "consolidation degraded for this project; the service continues"
                );
            }
        });
    }
    while let Some(joined) = drains.join_next().await {
        if let Err(join_error) = joined {
            // Same contract as the error arm above, one level out: a panic in one project's drain
            // must not end the loop for the other two.
            tracing::warn!(
                %join_error,
                "a consolidation task ended abnormally; the service continues"
            );
        }
    }
}

/// Drain one project: top up its backlog, then run jobs until it is idle or the provider is not.
///
/// Split out of the tick so it can be spawned. It takes a path rather than an open ledger because
/// each task opens its own SQLite connection — sharing one across concurrent tasks is the thing
/// this design is careful not to do.
async fn drain_project(
    ledger_path: &std::path::Path,
    project_id: brain_domain::ProjectId,
    worker: &ConsolidationWorker,
    llm: &dyn ConsolidationLlm,
) -> Result<()> {
    let mut ledger = EventLedger::open(ledger_path, project_id)?;
    // Chunk any uncovered backlog one bounded job at a time. Jobs are otherwise only enqueued when
    // capture ingests new events, so a project whose history was ingested before this loop existed
    // would never be consolidated at all.
    if let Err(error) = ledger.enqueue_event_threshold_job(1) {
        tracing::warn!(%error, "backlog enqueue failed");
    }
    for _ in 0..JOBS_PER_PROJECT_PER_TICK {
        match worker
            .run_once(
                &mut ledger,
                llm,
                time::OffsetDateTime::now_utc(),
                ConsolidationCrashPoint::None,
            )
            .await?
        {
            WorkerOutcome::Idle => break,
            // Stop this project's tick. The next job would reach the same unavailable provider,
            // and every extra call during a rate limit only lengthens it.
            //
            // Deliberately per-project, not global: the three projects back off independently, so
            // one hitting a limit does not stall the other two. That matters more now they run
            // concurrently — a shared backoff would have made the cap behave like the serial loop
            // it replaces the moment any one project got throttled.
            WorkerOutcome::ProviderUnavailable(_) => break,
            WorkerOutcome::Completed(_)
            | WorkerOutcome::RetryScheduled(_)
            | WorkerOutcome::DeadLetter(_)
            | WorkerOutcome::SimulatedCrash(_) => {}
        }
    }
    Ok(())
}

fn packet_from_events(job: &ConsolidationJob, events: Vec<StoredEvent>) -> EvidencePacket {
    let mut redactions = Vec::new();
    let events = events
        .into_iter()
        .map(|event| RedactedEvidence {
            event_id: event.event_id,
            event_type: event.event_type,
            occurred_at: event.occurred_at,
            payload: redact_value(event.payload, &mut redactions),
            raw: redact_value(event.raw, &mut redactions),
        })
        .collect();
    let mut seen = HashSet::new();
    redactions.retain(|entry| seen.insert((entry.category.clone(), entry.token_hash)));
    EvidencePacket {
        job_id: job.id,
        project_id: job.project_id,
        trigger: Some(job.reason.as_str().to_owned()),
        events,
        redactions,
        allowed_supersession_ids: Vec::new(),
    }
}

fn redact_value(
    value: serde_json::Value,
    manifest: &mut Vec<RedactionManifestEntry>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => {
            serde_json::Value::String(redact_string(&value, manifest))
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| redact_value(value, manifest))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_value(value, manifest)))
                .collect(),
        ),
        other => other,
    }
}

fn redact_string(value: &str, manifest: &mut Vec<RedactionManifestEntry>) -> String {
    let mut redacted = redact_captures(value, credential_assignment(), 1, "api_key", manifest);
    redacted = redact_captures(&redacted, secret_token(), 0, "api_key", manifest);
    redact_high_entropy(&redacted, manifest)
}

fn redact_captures(
    value: &str,
    pattern: &Regex,
    capture: usize,
    category: &str,
    manifest: &mut Vec<RedactionManifestEntry>,
) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    for captures in pattern.captures_iter(value) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        let Some(secret) = captures.get(capture) else {
            continue;
        };
        output.push_str(&value[cursor..secret.start()]);
        output.push_str(&format!("[REDACTED:{category}]"));
        output.push_str(&value[secret.end()..full.end()]);
        manifest.push(manifest_entry(category, secret.as_str()));
        cursor = full.end();
    }
    output.push_str(&value[cursor..]);
    output
}

fn redact_high_entropy(value: &str, manifest: &mut Vec<RedactionManifestEntry>) -> String {
    redact_captures(value, high_entropy_token(), 0, "high_entropy", manifest)
}

fn manifest_entry(category: &str, token: &str) -> RedactionManifestEntry {
    RedactionManifestEntry {
        category: category.to_owned(),
        token_hash: Sha256::digest(token.as_bytes()).into(),
    }
}

fn credential_assignment() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)(?:api[_-]?key|token|secret|password)\s*[:=]\s*([A-Za-z0-9_./+\-=]{8,})")
            .expect("credential regex is valid")
    })
}

fn secret_token() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{12,}\b").expect("secret regex is valid"))
}

fn high_entropy_token() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"\b[A-Za-z0-9+/=_-]{40,}\b").expect("entropy regex is valid"))
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;
    use brain_domain::{
        EventBatch, EventType, Harness, MemoryKind, NormalizedEvent, ProjectId, SourceCursor,
        WorktreeId,
    };
    use brain_store::ConsolidationReason;

    /// One provider call, as an interval: which project asked, and when it started and finished.
    #[derive(Clone, Copy, Debug)]
    struct Call {
        project: ProjectId,
        entered: std::time::Instant,
        left: std::time::Instant,
    }

    /// A proposer that is slow on purpose and records exactly when each call was in flight.
    ///
    /// The delay is the whole instrument: a fast fake would finish before the next task started
    /// and the intervals would never overlap, so the test would pass against the serial loop too.
    struct IntervalRecordingProposer {
        calls: std::sync::Mutex<Vec<Call>>,
    }

    #[async_trait::async_trait]
    impl ConsolidationLlm for IntervalRecordingProposer {
        async fn propose(
            &self,
            packet: &EvidencePacket,
        ) -> anyhow::Result<brain_context::ProposedMemoryBatch> {
            let entered = std::time::Instant::now();
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            self.calls.lock().expect("lock calls").push(Call {
                project: packet.project_id,
                entered,
                left: std::time::Instant::now(),
            });
            Ok(brain_context::ProposedMemoryBatch {
                memories: vec![brain_context::ProposedMemory {
                    kind: MemoryKind::Timeline,
                    title: "Fixture timeline".to_owned(),
                    content: "fixture".to_owned(),
                    valid_from: time::OffsetDateTime::UNIX_EPOCH,
                    confidence: 1.0,
                    evidence_ids: packet.events.iter().map(|event| event.event_id).collect(),
                    supersedes: Vec::new(),
                }],
            })
        }
    }

    fn overlaps(left: &Call, right: &Call) -> bool {
        left.entered < right.left && right.entered < left.left
    }

    /// A project with two queued jobs, on disk, because the drain opens ledgers by path.
    fn project_with_two_jobs(root: &std::path::Path, index: usize) -> crate::ServiceProjectConfig {
        let project_id = ProjectId(uuid::Uuid::now_v7());
        let ledger_path = root.join(format!("project-{index}.sqlite"));
        let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open ledger");
        for _ in 0..2 {
            let event_id = uuid::Uuid::now_v7();
            ledger
                .append_batch(&EventBatch {
                    source_id: format!("fixture-{index}"),
                    events: vec![NormalizedEvent {
                        event_id,
                        project_id,
                        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                        task_id: None,
                        harness: Harness::Codex,
                        native_session_id: format!("session-{index}"),
                        native_turn_id: None,
                        event_type: EventType::CheckpointAuthored,
                        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                        observed_at: time::OffsetDateTime::UNIX_EPOCH,
                        source_locator: format!("fixture-{index}"),
                        source_offset: 1,
                        source_schema: "fixture".to_owned(),
                        raw_hash: [index as u8; 32],
                        idempotency_key: uuid_bytes(event_id),
                        git_head: None,
                        git_branch: None,
                        payload: serde_json::json!({"content": "fixture"}),
                        raw: serde_json::json!({"content": "fixture"}),
                    }],
                    quarantined: Vec::new(),
                    capture_gaps: Vec::new(),
                    next_cursor: SourceCursor::byte_offset(1),
                })
                .expect("append event");
            ledger
                .enqueue_consolidation_job(
                    event_id,
                    event_id,
                    ConsolidationReason::ExplicitCheckpoint,
                )
                .expect("enqueue job");
        }
        crate::ServiceProjectConfig {
            project_root: root.join(format!("root-{index}")),
            project_id,
            worktree_id: WorktreeId(uuid::Uuid::now_v7()),
            ledger_path,
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        }
    }

    fn uuid_bytes(id: uuid::Uuid) -> [u8; 32] {
        let mut key = [0_u8; 32];
        key[..16].copy_from_slice(id.as_bytes());
        key
    }

    /// Separate ledgers drain together; one ledger is never asked twice at once.
    ///
    /// Both halves matter and they pull in opposite directions. Before this, the tick awaited each
    /// project in turn and each job in turn, so three projects and eight slots produced exactly one
    /// provider call at a time — a ~90 jobs/hour ceiling that every measurement landed under and
    /// nothing in the code said out loud. The fix must not overshoot into two writers on one SQLite
    /// file, which is why the second assertion is here and not left to review.
    #[tokio::test]
    async fn projects_consolidate_concurrently_but_never_one_ledger_twice() {
        let root = tempfile::tempdir().expect("tempdir");
        let projects: Vec<_> = (0..3)
            .map(|index| project_with_two_jobs(root.path(), index))
            .collect();
        let proposer = std::sync::Arc::new(IntervalRecordingProposer {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let llm: std::sync::Arc<dyn ConsolidationLlm> = proposer.clone();
        let worker = std::sync::Arc::new(ConsolidationWorker::new(
            "test-worker",
            time::Duration::seconds(30),
        ));
        let permits =
            std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONSOLIDATION));

        drain_all_projects(&projects, &worker, &llm, &permits).await;

        let calls = proposer.calls.lock().expect("read calls").clone();
        assert_eq!(calls.len(), 6, "every queued job should have been offered");

        let mut cross_project_overlaps = 0;
        for (index, left) in calls.iter().enumerate() {
            for right in &calls[index + 1..] {
                if !overlaps(left, right) {
                    continue;
                }
                assert_ne!(
                    left.project, right.project,
                    "two calls overlapped on one ledger — same-ledger concurrency is the thing \
                     this design rules out"
                );
                cross_project_overlaps += 1;
            }
        }
        assert!(
            cross_project_overlaps > 0,
            "no two projects were ever in flight together, which is what the serial loop did"
        );
    }
}

#[cfg(test)]
mod outage_tests {
    use super::provider_unavailable;

    #[test]
    fn a_rate_limit_is_an_outage_not_a_bad_job() {
        // Attempts back off as 1<<attempt seconds, so five are spent in about a minute. A rate
        // limit lasting hours would dead-letter every job it touched, none of which was ever
        // the problem.
        for text in [
            "GLM request failed with HTTP status 429 Too Many Requests",
            "GLM request failed: operation timed out",
            "error sending request: tcp connect error",
            "GLM request failed with HTTP status 503 Service Unavailable",
            "quota exceeded for this window",
        ] {
            assert!(
                provider_unavailable(&anyhow::anyhow!("{text}")),
                "should be treated as an outage: {text}"
            );
        }
    }

    #[test]
    fn a_malformed_proposal_is_a_real_failure_and_still_counts() {
        // The opposite mistake would be worse: a job that can never succeed would retry
        // forever, holding a slot and spending a provider call every time.
        for text in [
            "GLM message content does not match the proposed-memory schema",
            "unknown evidence ID 019fcd91 in provider output",
            "provider proposed more than 32 memories",
            "GLM API key is empty",
        ] {
            assert!(
                !provider_unavailable(&anyhow::anyhow!("{text}")),
                "should count as a real failure: {text}"
            );
        }
    }
}
