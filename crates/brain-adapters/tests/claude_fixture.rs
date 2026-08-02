use std::io::Write;

use brain_adapters::{
    ClaudeAdapter, NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor,
};
use brain_domain::{EventType, ProjectId, SourceCursor, WorktreeId};

#[test]
fn claude_adapter_preserves_unknown_events() {
    let adapter = ClaudeAdapter::new(fixture_root());
    let source = SourceDescriptor::file(fixture("unknown-event.jsonl"));
    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read unknown fixture")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };

    let events = adapter
        .normalize(&batch.records[0], &normalize_context())
        .expect("normalize unknown event");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::SchemaUnknown);
    assert_eq!(events[0].raw["type"], "future-event");
    assert_eq!(events[0].raw["futureField"]["kept"], true);
}

#[test]
fn partial_final_line_is_not_read_or_committed() {
    let temp = tempfile::tempdir().expect("create truncated fixture root");
    let path = temp.path().join("truncated.jsonl");
    let mut bytes = std::fs::read(fixture("truncated.jsonl")).expect("read fixture source");
    assert_eq!(bytes.pop(), Some(b'\n'));
    let mut file = std::fs::File::create(&path).expect("create runtime fixture");
    file.write_all(&bytes).expect("write runtime fixture");
    file.sync_all().expect("flush runtime fixture");
    let expected_offset = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|index| index as u64 + 1)
        .expect("fixture contains first newline");

    let adapter = ClaudeAdapter::new(temp.path());
    let source = SourceDescriptor::file(path);
    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read truncated fixture")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };

    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.next_cursor.byte_offset, expected_offset);
    assert_eq!(batch.last_complete_newline, expected_offset);
}

#[test]
fn known_claude_shapes_emit_primary_and_tool_events_in_source_order() {
    let adapter = ClaudeAdapter::new(fixture_root());
    let source = SourceDescriptor::file(fixture("session.jsonl"));
    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read session fixture")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };
    let events = batch
        .records
        .iter()
        .flat_map(|record| {
            adapter
                .normalize(record, &normalize_context())
                .expect("normalize known Claude record")
        })
        .collect::<Vec<_>>();

    let event_types = events
        .iter()
        .map(|event| event.event_type.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            EventType::UserPrompted,
            EventType::AgentResponded,
            EventType::ToolRequested,
            EventType::ToolCompleted,
            EventType::SystemObserved,
            EventType::AttachmentObserved,
            EventType::QueueOperationObserved,
            EventType::ModeChanged,
            EventType::SessionRelocated,
        ]
    );
    let tool_request = events
        .iter()
        .find(|event| event.event_type == EventType::ToolRequested)
        .expect("tool request event");
    assert_eq!(tool_request.payload["name"], "Read");
    let keys = events
        .iter()
        .map(|event| event.idempotency_key)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(keys.len(), events.len());
}

#[test]
fn a_complete_malformed_line_becomes_unknown_evidence_with_parse_diagnostics() {
    let adapter = ClaudeAdapter::new(fixture_root());
    let source = SourceDescriptor::file(fixture("malformed.jsonl"));
    let batch = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read malformed fixture")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };

    let events = adapter
        .normalize(&batch.records[0], &normalize_context())
        .expect("normalize malformed record");

    assert_eq!(events[0].event_type, EventType::SchemaUnknown);
    assert!(events[0].payload["parse_error"].is_string());
    assert_eq!(
        events[0].raw["raw_text"],
        "{\"type\":\"user\",\"sessionId\":\"broken\""
    );
}

#[test]
fn replacing_a_source_file_restarts_at_zero_and_reports_rotation() {
    let temp = tempfile::tempdir().expect("create rotation fixture root");
    let path = temp.path().join("session.jsonl");
    std::fs::copy(fixture("unknown-event.jsonl"), &path).expect("copy original source");
    let adapter = ClaudeAdapter::new(temp.path());
    let source = SourceDescriptor::file(&path);
    let first = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read original source")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected batch, got {other:?}"),
    };

    std::fs::remove_file(&path).expect("remove original source");
    std::fs::copy(fixture("session.jsonl"), &path).expect("copy replacement source");
    let replacement = match adapter
        .read_increment(&source, &first.next_cursor)
        .expect("read replacement source")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected replacement batch, got {other:?}"),
    };

    assert!(replacement.rotation.is_some());
    assert_eq!(replacement.records[0].byte_offset, 0);
    assert_eq!(replacement.records.len(), 8);
}

#[test]
fn schema_fingerprint_changes_when_a_native_record_type_changes() {
    let temp = tempfile::tempdir().expect("create fingerprint fixture root");
    let user_path = temp.path().join("user.jsonl");
    let future_path = temp.path().join("future.jsonl");
    std::fs::write(&user_path, "{\"type\":\"user\",\"sessionId\":\"s\"}\n")
        .expect("write user fixture");
    std::fs::write(
        &future_path,
        "{\"type\":\"future-event\",\"sessionId\":\"s\"}\n",
    )
    .expect("write future fixture");
    let adapter = ClaudeAdapter::new(temp.path());

    let user = adapter
        .fingerprint(&SourceDescriptor::file(user_path))
        .expect("fingerprint user shape");
    let future = adapter
        .fingerprint(&SourceDescriptor::file(future_path))
        .expect("fingerprint future shape");

    assert_ne!(user, future);
}

#[test]
fn committed_cursor_reads_only_newly_appended_complete_records() {
    let temp = tempfile::tempdir().expect("create replay fixture root");
    let path = temp.path().join("session.jsonl");
    std::fs::copy(fixture("unknown-event.jsonl"), &path).expect("copy initial source");
    let adapter = ClaudeAdapter::new(temp.path());
    let source = SourceDescriptor::file(&path);
    let first = match adapter
        .read_increment(&source, &SourceCursor::start())
        .expect("read initial source")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected initial batch, got {other:?}"),
    };
    let first_end = first.next_cursor.byte_offset;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open source for append");
    file.write_all(
        b"{\"type\":\"mode\",\"mode\":\"acceptEdits\",\"sessionId\":\"session-unknown\"}\n",
    )
    .expect("append complete record");
    file.sync_all().expect("flush appended record");

    let second = match adapter
        .read_increment(&source, &first.next_cursor)
        .expect("read appended source")
    {
        ReadOutcome::Batch(batch) => batch,
        other => panic!("expected appended batch, got {other:?}"),
    };

    assert_eq!(second.records.len(), 1);
    assert_eq!(second.records[0].byte_offset, first_end);
    assert!(matches!(
        adapter
            .read_increment(&source, &second.next_cursor)
            .expect("read unchanged source"),
        ReadOutcome::NoChange
    ));
}

#[test]
fn discovery_recurses_for_jsonl_and_ignores_other_files() {
    let temp = tempfile::tempdir().expect("create discovery fixture root");
    let nested = temp.path().join("project").join("nested");
    std::fs::create_dir_all(&nested).expect("create nested project path");
    std::fs::copy(fixture("unknown-event.jsonl"), nested.join("session.jsonl"))
        .expect("copy JSONL source");
    std::fs::write(nested.join("notes.txt"), "not a transcript\n").expect("write ignored file");

    let sources = ClaudeAdapter::new(temp.path())
        .discover()
        .expect("discover Claude sources");

    assert_eq!(sources.len(), 1);
    assert!(sources[0].path.ends_with("session.jsonl"));
}

fn fixture_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("claude")
}

fn fixture(name: &str) -> std::path::PathBuf {
    fixture_root().join(name)
}

fn normalize_context() -> NormalizeContext {
    NormalizeContext {
        project_id: ProjectId(uuid::Uuid::now_v7()),
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        source_schema: "claude-jsonl:fixture".to_owned(),
    }
}
