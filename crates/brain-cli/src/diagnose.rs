use std::path::Path;

use anyhow::Result;
use brain_domain::{ProjectId, SchemaDriftRecord};
use brain_service::ServiceLaunchConfig;
use brain_store::{BASIC_MEMORY_PINNED_VERSION, EventLedger, MarkdownProjector};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, serde::Serialize)]
pub struct DiagnosticBundle {
    pub format_version: u32,
    pub generated_at: time::OffsetDateTime,
    pub project_id: ProjectId,
    pub persisted_events: u64,
    pub memory_records: u64,
    pub memory_versions: u64,
    pub pending_note_reviews: u64,
    pub markdown_projection_valid: Option<bool>,
    pub basic_memory_pinned_version: &'static str,
    pub active_schema_drifts: u64,
    pub schema_drifts: Vec<RedactedSchemaDrift>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RedactedSchemaDrift {
    pub diagnostic_id: uuid::Uuid,
    pub source_ref: String,
    pub expected_fingerprint: String,
    pub observed_fingerprint: String,
    pub cursor: RedactedCursor,
    pub sample_hash: String,
    pub reason_code: &'static str,
    pub observed_at: time::OffsetDateTime,
    pub resolved_at: Option<time::OffsetDateTime>,
    pub active: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RedactedCursor {
    pub byte_offset: u64,
    pub file_identity_ref: Option<String>,
    pub native_position_keys: Vec<String>,
}

pub fn read_diagnostics(
    brain_home: impl AsRef<Path>,
    project: Option<ProjectId>,
) -> Result<DiagnosticBundle> {
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home.as_ref()))?;
    let project = config.project(project)?;
    let ledger = EventLedger::open(&project.ledger_path, project.project_id)?;
    let active_schema_drifts = ledger.active_schema_drift_count()?;
    let schema_drifts = ledger
        .schema_drifts()?
        .into_iter()
        .map(redact_drift)
        .collect();
    let manifest_path = brain_home
        .as_ref()
        .join("vault")
        .join("projects")
        .join(project.project_id.0.to_string())
        .join("generated")
        .join("current.json");
    let markdown_projection_valid = if manifest_path.is_file() {
        Some(
            MarkdownProjector::new(brain_home.as_ref().join("vault"))
                .verify_project(project.project_id)
                .is_ok_and(|report| report.valid),
        )
    } else {
        None
    };
    Ok(DiagnosticBundle {
        format_version: 1,
        generated_at: time::OffsetDateTime::now_utc(),
        project_id: project.project_id,
        persisted_events: ledger.event_count()?,
        memory_records: ledger.memory_count()?,
        memory_versions: ledger.memory_version_count()?,
        pending_note_reviews: ledger.note_review_count()?,
        markdown_projection_valid,
        basic_memory_pinned_version: BASIC_MEMORY_PINNED_VERSION,
        active_schema_drifts,
        schema_drifts,
    })
}

fn redact_drift(record: SchemaDriftRecord) -> RedactedSchemaDrift {
    let mut native_position_keys = record
        .cursor
        .native_position
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    native_position_keys.sort();
    RedactedSchemaDrift {
        diagnostic_id: record.diagnostic_id,
        source_ref: hash_ref(&record.source_id),
        expected_fingerprint: record.expected_fingerprint,
        observed_fingerprint: record.observed_fingerprint,
        cursor: RedactedCursor {
            byte_offset: record.cursor.byte_offset,
            file_identity_ref: record.cursor.file_identity.as_deref().map(hash_ref),
            native_position_keys,
        },
        sample_hash: hex::encode(record.sample_hash),
        reason_code: "adapter_schema_differs_from_reviewed_profile",
        observed_at: record.observed_at,
        resolved_at: record.resolved_at,
        active: record.resolved_at.is_none(),
    }
}

fn hash_ref(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}
