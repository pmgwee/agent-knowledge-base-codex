use std::collections::HashSet;
use std::sync::OnceLock;

use anyhow::Result;
use brain_domain::{EventType, MemoryRecord, ProjectId};
use brain_store::{ConsolidationJob, EventLedger, JobStatus, RedactionManifestEntry, StoredEvent};
use regex::Regex;
use sha2::{Digest, Sha256};

pub trait MemoryProposer {
    fn propose(&self, packet: &EvidencePacket) -> Result<Vec<MemoryRecord>>;
}

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
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct EvidencePacket {
    pub job_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub events: Vec<RedactedEvidence>,
    #[serde(skip)]
    pub redactions: Vec<RedactionManifestEntry>,
}

impl EvidencePacket {
    pub fn serialized(&self) -> String {
        serde_json::to_string(self).expect("evidence packet serialization is infallible")
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RedactedEvidence {
    pub event_id: uuid::Uuid,
    pub event_type: EventType,
    pub occurred_at: time::OffsetDateTime,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
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

    pub fn run_once(
        &self,
        ledger: &mut EventLedger,
        proposer: &dyn MemoryProposer,
        now: time::OffsetDateTime,
        crash_point: ConsolidationCrashPoint,
    ) -> Result<WorkerOutcome> {
        let Some(job) =
            ledger.lease_consolidation_job(&self.worker_id, now, self.lease_duration)?
        else {
            return Ok(WorkerOutcome::Idle);
        };
        let events = ledger.events_between(job.first_event_id, job.last_event_id)?;
        let packet = EvidencePacket::from_events(&job, events);
        ledger.record_redaction_manifest(job.id, &packet.redactions)?;
        let proposed = match proposer.propose(&packet) {
            Ok(proposed) => proposed,
            Err(error) => return self.fail(ledger, &job, &error.to_string(), now),
        };
        for memory in &proposed {
            if let Err(error) = ledger.append_memory(memory) {
                return self.fail(ledger, &job, &error.to_string(), now);
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

impl EvidencePacket {
    fn from_events(job: &ConsolidationJob, events: Vec<StoredEvent>) -> Self {
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
        Self {
            job_id: job.id,
            project_id: job.project_id,
            events,
            redactions,
        }
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
