use std::sync::Arc;

use anyhow::Result;
use brain_adapters::{
    ClaudeAdapter, NormalizeContext, RawRecord, ReadOutcome, SchemaDrift, SchemaFingerprint,
    SourceAdapter, SourceDescriptor,
};
use brain_domain::{NormalizedEvent, ProjectId, SourceCursor, WorktreeId};
use brain_service::{CaptureBinding, CaptureSupervisor};
use brain_store::EventLedger;

struct DriftAdapter;

impl SourceAdapter for DriftAdapter {
    fn name(&self) -> &'static str {
        "drift-fixture"
    }

    fn discover(&self) -> Result<Vec<SourceDescriptor>> {
        Ok(Vec::new())
    }

    fn fingerprint(&self, _source: &SourceDescriptor) -> Result<SchemaFingerprint> {
        Ok(SchemaFingerprint("fixture:observed".to_owned()))
    }

    fn read_increment(
        &self,
        source: &SourceDescriptor,
        _cursor: &SourceCursor,
    ) -> Result<ReadOutcome> {
        Ok(ReadOutcome::SchemaDrift(SchemaDrift {
            source_id: source.source_id.clone(),
            expected: SchemaFingerprint("fixture:expected".to_owned()),
            observed: SchemaFingerprint("fixture:observed".to_owned()),
            sample_hash: [9; 32],
        }))
    }

    fn normalize(
        &self,
        _record: &RawRecord,
        _context: &NormalizeContext,
    ) -> Result<Vec<NormalizedEvent>> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn drift_pauses_only_the_affected_source_and_survives_restart() {
    let temp = tempfile::tempdir().expect("create containment fixture");
    let drifting_path = temp.path().join("drifting.db");
    std::fs::write(&drifting_path, b"fixture").expect("write drift source");
    let healthy_path = temp.path().join("healthy.jsonl");
    std::fs::write(
        &healthy_path,
        "{\"type\":\"future-event\",\"sessionId\":\"healthy-session\"}\n",
    )
    .expect("write healthy source");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.sqlite");
    let drift_source = SourceDescriptor::file(&drifting_path);
    let healthy_source = SourceDescriptor::file(&healthy_path);

    let supervisor = make_supervisor(
        &ledger_path,
        project_id,
        worktree_id,
        drift_source.clone(),
        healthy_source.clone(),
        temp.path(),
    );
    supervisor
        .capture_once()
        .await
        .expect("one drifting source must not fail reconciliation");

    assert_eq!(
        supervisor
            .event_count(project_id)
            .expect("count healthy event"),
        1
    );
    let health = supervisor.health().expect("read health");
    let drift_health = health
        .sources
        .get(&drift_source.source_id)
        .expect("drift health");
    assert!(
        drift_health
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("schema drift"))
    );
    let healthy_health = health
        .sources
        .get(&healthy_source.source_id)
        .expect("healthy source health");
    assert!(healthy_health.last_error.is_none());
    assert!(healthy_health.last_cursor.byte_offset > 0);
    drop(supervisor);

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen ledger");
    let persisted = ledger
        .active_schema_drift(&drift_source.source_id)
        .expect("read persisted drift")
        .expect("drift remains active");
    assert_eq!(persisted.expected_fingerprint, "fixture:expected");
    assert_eq!(persisted.observed_fingerprint, "fixture:observed");
    assert_eq!(persisted.cursor, SourceCursor::start());
    assert_eq!(persisted.sample_hash, [9; 32]);
    drop(ledger);

    let restarted = make_supervisor(
        &ledger_path,
        project_id,
        worktree_id,
        drift_source.clone(),
        healthy_source,
        temp.path(),
    );
    let restarted_health = restarted.health().expect("read restarted health");
    let restarted_drift = restarted_health
        .sources
        .get(&drift_source.source_id)
        .expect("restarted drift health");
    assert!(restarted_drift.active_schema_drift.is_some());
    assert!(restarted_drift.last_error.is_some());
    assert!(!restarted_health.is_healthy());
}

fn make_supervisor(
    ledger_path: &std::path::Path,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    drift_source: SourceDescriptor,
    healthy_source: SourceDescriptor,
    root: &std::path::Path,
) -> CaptureSupervisor {
    let context = NormalizeContext {
        project_id,
        worktree_id,
        source_schema: "fixture".to_owned(),
    };
    CaptureSupervisor::new(vec![
        CaptureBinding::new(
            Arc::new(DriftAdapter),
            drift_source,
            context.clone(),
            ledger_path,
        ),
        CaptureBinding::new(
            Arc::new(ClaudeAdapter::new(root)),
            healthy_source,
            context,
            ledger_path,
        ),
    ])
    .expect("create supervisor")
}
