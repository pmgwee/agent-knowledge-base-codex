use brain_domain::{ProjectId, SchemaDriftRecord, SourceCursor};
use brain_store::EventLedger;

#[test]
fn schema_drift_is_durable_without_advancing_the_source_cursor() {
    let temp = tempfile::tempdir().expect("create schema drift fixture");
    let path = temp.path().join("events.sqlite");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let source_id = "hermes:fixture";
    let cursor = SourceCursor::for_native(
        41,
        "db-file-identity".to_owned(),
        serde_json::json!({"message_id": 41}),
    );

    let mut ledger = EventLedger::open(&path, project_id).expect("open ledger");
    ledger
        .record_schema_drift(&SchemaDriftRecord {
            diagnostic_id: uuid::Uuid::now_v7(),
            source_id: source_id.to_owned(),
            expected_fingerprint: "hermes:v22".to_owned(),
            observed_fingerprint: "hermes:v23".to_owned(),
            cursor: cursor.clone(),
            sample_hash: [7; 32],
            reason: "installed schema differs from reviewed profile".to_owned(),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            resolved_at: None,
        })
        .expect("persist schema drift");

    assert_eq!(
        ledger.cursor(source_id).expect("read cursor"),
        SourceCursor::start()
    );
    drop(ledger);

    let mut reopened = EventLedger::open(&path, project_id).expect("reopen ledger");
    let active = reopened
        .active_schema_drift(source_id)
        .expect("read active drift")
        .expect("active drift exists");
    assert_eq!(active.expected_fingerprint, "hermes:v22");
    assert_eq!(active.observed_fingerprint, "hermes:v23");
    assert_eq!(active.cursor, cursor);
    assert_eq!(active.sample_hash, [7; 32]);
    assert_eq!(
        reopened.active_schema_drift_count().expect("count drift"),
        1
    );

    reopened
        .resolve_schema_drift(source_id, time::OffsetDateTime::UNIX_EPOCH)
        .expect("resolve drift");
    assert!(
        reopened
            .active_schema_drift(source_id)
            .expect("read resolved drift")
            .is_none()
    );
    assert_eq!(reopened.schema_drifts().expect("list history").len(), 1);
}
