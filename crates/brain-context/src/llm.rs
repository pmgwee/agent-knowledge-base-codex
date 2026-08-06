use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use brain_domain::{
    Authority, EventType, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId,
};
use brain_store::RedactionManifestEntry;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, serde::Serialize)]
pub struct EvidencePacket {
    pub job_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub events: Vec<RedactedEvidence>,
    #[serde(skip)]
    pub redactions: Vec<RedactionManifestEntry>,
    #[serde(skip)]
    pub allowed_supersession_ids: Vec<uuid::Uuid>,
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedMemoryBatch {
    pub memories: Vec<ProposedMemory>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedMemory {
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    #[serde(with = "time::serde::rfc3339")]
    pub valid_from: time::OffsetDateTime,
    pub confidence: f32,
    pub evidence_ids: Vec<uuid::Uuid>,
    /// Absent, `[]`, and `null` all mean "supersedes nothing".
    ///
    /// A model asked for an array it has no members for will sometimes write `null`, and
    /// `Vec` alone rejects that — discarding an otherwise valid batch over an empty field.
    /// Tolerated here because the meaning is unambiguous; a wrong `kind` is not, and is still
    /// rejected.
    #[serde(default, deserialize_with = "null_as_default")]
    pub supersedes: Vec<uuid::Uuid>,
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    use serde::Deserialize as _;
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[async_trait]
pub trait ConsolidationLlm: Send + Sync {
    async fn propose(&self, packet: &EvidencePacket) -> Result<ProposedMemoryBatch>;
}

/// What survived validation, and what did not.
///
/// Rejections are returned rather than swallowed so a caller can record why a proposal was
/// dropped. Without that, missing memories look like a model that had nothing to say.
#[derive(Clone, Debug, Default)]
pub struct ValidatedBatch {
    pub accepted: Vec<MemoryRecord>,
    pub rejected: Vec<String>,
}

/// Validate a proposed batch, keeping the memories that hold up.
///
/// Rejection is **per memory**, not per batch. A provider that returns six sound memories and
/// one citing an invented event id previously lost all seven, and because the request is made
/// at `temperature: 0` the retry produced the identical output — so the job burned its five
/// attempts and dead-lettered, discarding work that was never in question.
///
/// The integrity rules are unchanged and still absolute: a memory citing evidence outside its
/// packet, or proposing a global preference, never becomes a record. It is only the blast
/// radius that shrinks, from the batch to the offending memory.
pub fn validate_proposed_batch(
    packet: &EvidencePacket,
    batch: ProposedMemoryBatch,
) -> Result<ValidatedBatch> {
    ensure!(
        batch.memories.len() <= 32,
        "provider proposed more than 32 memories"
    );
    let evidence_ids = packet
        .events
        .iter()
        .map(|event| event.event_id)
        .collect::<HashSet<_>>();
    let allowed_supersession = packet
        .allowed_supersession_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let earliest = packet
        .events
        .iter()
        .map(|event| event.occurred_at)
        .min()
        .context("cannot validate memory without evidence events")?;
    let latest = packet
        .events
        .iter()
        .map(|event| event.occurred_at)
        .max()
        .context("cannot validate memory without evidence events")?;

    let mut validated = ValidatedBatch::default();
    for (index, proposed) in batch.memories.into_iter().enumerate() {
        let title = proposed.title.clone();
        let outcome = (|| {
            ensure!(
                proposed.kind != MemoryKind::Preference,
                "provider cannot propose global preferences"
            );
            ensure!(
                !proposed.title.trim().is_empty() && proposed.title.len() <= 300,
                "proposed memory title is empty or oversized"
            );
            ensure!(
                !proposed.content.trim().is_empty() && proposed.content.len() <= 20_000,
                "proposed memory content is empty or oversized"
            );
            ensure!(
                proposed.confidence.is_finite() && (0.0..=1.0).contains(&proposed.confidence),
                "proposed memory confidence is invalid"
            );
            ensure!(
                proposed.valid_from >= earliest - time::Duration::days(365)
                    && proposed.valid_from <= latest + time::Duration::minutes(5),
                "proposed memory timestamp is outside the evidence interval"
            );
            ensure!(
                !proposed.evidence_ids.is_empty(),
                "proposed memory has no evidence citations"
            );
            for evidence_id in &proposed.evidence_ids {
                ensure!(
                    evidence_ids.contains(evidence_id),
                    "unknown evidence ID {evidence_id} in provider output"
                );
            }
            for superseded in &proposed.supersedes {
                ensure!(
                    allowed_supersession.contains(superseded),
                    "unknown supersession ID {superseded} in provider output"
                );
            }
            let index = u32::try_from(index)?;
            Ok(MemoryRecord {
                id: deterministic_id(packet.job_id, index, b"memory"),
                version_id: deterministic_id(packet.job_id, index, b"version"),
                scope: MemoryScope::Project(packet.project_id),
                worktree_id: None,
                task_id: None,
                kind: proposed.kind,
                title: proposed.title,
                content: proposed.content,
                valid_from: proposed.valid_from,
                valid_to: None,
                recorded_at: latest,
                confidence: proposed.confidence,
                authority: Authority::DerivedMemory,
                evidence_ids: proposed.evidence_ids,
                supersedes: proposed.supersedes,
                status: MemoryStatus::Current,
            })
        })();
        match outcome {
            Ok(record) => validated.accepted.push(record),
            Err(error) => validated
                .rejected
                .push(format!("{title:?}: {error}", title = truncate(&title, 80))),
        }
    }
    Ok(validated)
}

/// Keep a rejection reason readable when a provider returns a very long title.
fn truncate(text: &str, limit: usize) -> String {
    truncate_for_error(text, limit)
}

/// Bound provider text quoted into an error. Errors land in job records and logs, so an
/// unbounded quote of a malformed response would bury the failure it is meant to explain.
pub fn truncate_for_error(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

fn deterministic_id(seed: uuid::Uuid, index: u32, label: &[u8]) -> uuid::Uuid {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher.update(index.to_le_bytes());
    hasher.update(label);
    let hash = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes)
}
