use std::io::Write;

use brain_adapters::{
    CodexAdapter, NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor,
};
use brain_domain::{EventType, Harness, ProjectId, SourceCursor, WorktreeId};

#[test]
fn codex_maps_session_turn_tool_and_compaction_events() {
    let rollout = normalize_fixture("rollout.jsonl");
    assert!(has_type(&rollout, EventType::SessionStarted));
    assert!(has_type(&rollout, EventType::UserPrompted));
    assert!(has_type(&rollout, EventType::AgentResponded));
    assert!(has_type(&rollout, EventType::ToolRequested));
    assert!(has_type(&rollout, EventType::ToolFailed));
    assert!(has_type(&rollout, EventType::SystemObserved));
    assert!(rollout.iter().all(|event| event.harness == Harness::Codex));

    let compacted = normalize_fixture("compacted.jsonl");
    assert!(has_type(&compacted, EventType::SessionStarted));
    assert!(has_type(&compacted, EventType::UserPrompted));
    assert!(has_type(&compacted, EventType::SessionCompacted));
}

#[test]
fn encrypted_reasoning_is_retained_but_never_projected_as_reasoning_text() {
    let events = normalize_fixture("encrypted-reasoning.jsonl");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::OpaqueEvidence);
    assert!(events[0].payload.get("reasoning_text").is_none());
    assert!(events[0].payload.get("encrypted_content").is_none());
    assert_eq!(events[0].payload["retention"], "raw-only");
    assert_eq!(
        events[0].raw["payload"]["encrypted_content"],
        "ENCRYPTED_REDACTED_FIXTURE_BLOB"
    );
    assert!(events[0].raw_hash.iter().any(|byte| *byte != 0));
}

#[test]
fn unknown_codex_records_are_preserved_without_guessing() {
    let events = normalize_fixture("unknown-event.jsonl");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::SchemaUnknown);
    assert_eq!(events[0].raw["type"], "future_rollout_record");
    assert_eq!(events[0].raw["payload"]["future_field"]["kept"], true);
}

#[test]
fn partial_final_record_does_not_advance_the_codex_cursor() {
    let temp = tempfile::tempdir().expect("create Codex partial fixture");
    let path = temp.path().join("rollout-partial.jsonl");
    let complete = std::fs::read(fixture("unknown-event.jsonl")).expect("read complete fixture");
    let mut file = std::fs::File::create(&path).expect("create runtime rollout");
    file.write_all(&complete).expect("write complete record");
    file.write_all(b"{\"type\":\"event_msg\",\"payload\":")
        .expect("write partial record");
    file.sync_all().expect("flush partial rollout");
    let adapter = CodexAdapter::new(vec![temp.path().to_path_buf()]);
    let source = SourceDescriptor::file(path);

    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read partial rollout")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };

    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.next_cursor.byte_offset, complete.len() as u64);
}

#[test]
fn replacing_a_codex_rollout_reports_rotation_and_restarts_at_zero() {
    let temp = tempfile::tempdir().expect("create Codex rotation fixture");
    let path = temp.path().join("rollout-rotation.jsonl");
    std::fs::copy(fixture("unknown-event.jsonl"), &path).expect("copy first rollout");
    let adapter = CodexAdapter::new(vec![temp.path().to_path_buf()]);
    let source = SourceDescriptor::file(&path);
    let first = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read first rollout")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected first batch, got {other:?}"),
    };

    std::fs::remove_file(&path).expect("remove first rollout");
    std::fs::copy(fixture("rollout.jsonl"), &path).expect("copy replacement rollout");
    let replacement = match adapter
        .read_increment(&source, &first.next_cursor)
        .expect("read replacement rollout")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected replacement batch, got {other:?}"),
    };

    assert!(replacement.rotation.is_some());
    assert_eq!(replacement.records[0].byte_offset, 0);
    assert_eq!(replacement.records.len(), 7);
}

#[test]
fn discovery_is_limited_to_configured_roots_and_rollout_files() {
    let temp = tempfile::tempdir().expect("create Codex discovery fixture");
    let configured = temp.path().join("configured");
    let foreign = temp.path().join("foreign");
    std::fs::create_dir_all(configured.join("nested")).expect("create configured root");
    std::fs::create_dir_all(&foreign).expect("create foreign root");
    std::fs::copy(
        fixture("rollout.jsonl"),
        configured.join("nested").join("rollout-fixture.jsonl"),
    )
    .expect("copy configured rollout");
    std::fs::copy(
        fixture("rollout.jsonl"),
        configured.join("nested").join("not-a-rollout.jsonl"),
    )
    .expect("copy ignored JSONL");
    std::fs::copy(
        fixture("rollout.jsonl"),
        foreign.join("rollout-foreign.jsonl"),
    )
    .expect("copy foreign rollout");

    let sources = CodexAdapter::new(vec![configured])
        .discover()
        .expect("discover Codex rollouts");

    assert_eq!(sources.len(), 1);
    assert!(sources[0].path.ends_with("rollout-fixture.jsonl"));
}

fn normalize_fixture(name: &str) -> Vec<brain_domain::NormalizedEvent> {
    let adapter = CodexAdapter::new(vec![fixture_root()]);
    let source = SourceDescriptor::file(fixture(name));
    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read Codex fixture")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };
    batch
        .records
        .iter()
        .flat_map(|record| {
            adapter
                .normalize(record, &normalize_context())
                .expect("normalize Codex record")
        })
        .collect()
}

fn has_type(events: &[brain_domain::NormalizedEvent], event_type: EventType) -> bool {
    events.iter().any(|event| event.event_type == event_type)
}

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("codex")
}

fn fixture(name: &str) -> std::path::PathBuf {
    fixture_root().join(name)
}

fn normalize_context() -> NormalizeContext {
    NormalizeContext {
        project_id: ProjectId(uuid::Uuid::now_v7()),
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        source_schema: "codex-rollout:fixture".to_owned(),
    }
}
