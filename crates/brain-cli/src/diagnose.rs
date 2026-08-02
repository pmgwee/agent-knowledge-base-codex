use std::path::Path;

use anyhow::Result;
use brain_domain::{ProjectId, SchemaDriftRecord};
use brain_service::ServiceLaunchConfig;
use brain_store::EventLedger;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, serde::Serialize)]
pub struct DiagnosticBundle {
    pub format_version: u32,
    pub generated_at: time::OffsetDateTime,
    pub project_id: ProjectId,
    pub persisted_events: u64,
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
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?;
    let project = config.project(project)?;
    let ledger = EventLedger::open(&project.ledger_path, project.project_id)?;
    let active_schema_drifts = ledger.active_schema_drift_count()?;
    let schema_drifts = ledger
        .schema_drifts()?
        .into_iter()
        .map(redact_drift)
        .collect();
    Ok(DiagnosticBundle {
        format_version: 1,
        generated_at: time::OffsetDateTime::now_utc(),
        project_id: project.project_id,
        persisted_events: ledger.event_count()?,
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
