use brain_cli::{RegisterOptions, read_diagnostics, register_project};
use brain_domain::{SchemaDriftRecord, SourceCursor};
use brain_store::EventLedger;

#[test]
fn diagnostic_bundle_reports_drift_without_leaking_source_paths_or_native_values() {
    let temp = tempfile::tempdir().expect("create diagnostic fixture");
    let brain_home = temp.path().join("brain");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let registration = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: None,
    })
    .expect("register project");
    let secret_source = r"file:C:\Users\person\private\session.jsonl";
    let mut ledger =
        EventLedger::open(&registration.ledger_path, registration.project_id).expect("open ledger");
    ledger
        .record_schema_drift(&SchemaDriftRecord {
            diagnostic_id: uuid::Uuid::now_v7(),
            source_id: secret_source.to_owned(),
            expected_fingerprint: "hermes:v22".to_owned(),
            observed_fingerprint: "hermes:v23".to_owned(),
            cursor: SourceCursor::for_native(
                73,
                "sensitive-file-identity".to_owned(),
                serde_json::json!({"session_id": "private-session", "message_id": 73}),
            ),
            sample_hash: [4; 32],
            reason: "adapter schema differs from its reviewed profile".to_owned(),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            resolved_at: None,
        })
        .expect("record drift");
    drop(ledger);

    let bundle = read_diagnostics(&brain_home, Some(registration.project_id))
        .expect("build diagnostic bundle");
    assert_eq!(bundle.active_schema_drifts, 1);
    assert_eq!(bundle.schema_drifts.len(), 1);
    let drift = &bundle.schema_drifts[0];
    assert_eq!(drift.cursor.byte_offset, 73);
    assert_eq!(
        drift.cursor.native_position_keys,
        vec!["message_id", "session_id"]
    );
    assert!(drift.source_ref.starts_with("sha256:"));

    let json = serde_json::to_string_pretty(&bundle).expect("serialize bundle");
    assert!(!json.contains(secret_source));
    assert!(!json.contains("private-session"));
    assert!(!json.contains("sensitive-file-identity"));
    assert!(!json.contains("Users\\\\person"));
}
