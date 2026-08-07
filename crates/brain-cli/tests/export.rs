//! Getting the data out, with its evidence attached.

use brain_cli::{ExportFormat, export_project};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn an_export_carries_the_evidence_its_memories_cite() {
    // The property that makes this an export rather than a dump. A memory without the turns it
    // cites is exactly the unsourced claim this system exists to avoid, and it would be produced
    // at the one moment nothing downstream can resolve it.
    let (mut ledger, project, worktree) = fixture();
    let first = append_event(&mut ledger, project, worktree, 1, "picked jose for Edge");
    let second = append_event(&mut ledger, project, worktree, 2, "ran the auth tests");
    let shared = memory(project, worktree, "Auth library", vec![first, second]);
    let other = memory(project, worktree, "Test coverage", vec![second]);
    ledger.append_memory(&shared).expect("append first");
    ledger.append_memory(&other).expect("append second");

    let temp = tempfile::tempdir().expect("export dir");
    let report = export_project(&ledger, project, temp.path(), ExportFormat::Both, None)
        .expect("export project");

    assert_eq!(report.memories, 2);
    assert_eq!(
        report.events, 2,
        "an event cited twice is exported once, not duplicated"
    );
    assert_eq!(report.memories_missing_evidence, 0);
    assert!(report.bytes > 0);

    let events: Vec<serde_json::Value> = std::fs::read_to_string(temp.path().join("events.jsonl"))
        .expect("read events")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a record"))
        .collect();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .any(|event| event["payload"]["content"] == "picked jose for Edge"),
        "the turn itself travels with the claim, not just its id"
    );

    let memories = std::fs::read_to_string(temp.path().join("memories.jsonl")).expect("read");
    assert_eq!(memories.lines().count(), 2, "one record per line");

    let index = std::fs::read_to_string(temp.path().join("index.md")).expect("read index");
    assert!(index.contains("Auth library"));
    assert!(
        temp.path()
            .join("memories")
            .join(shared.projection_file_name())
            .is_file(),
        "markdown export is one readable file per memory"
    );
}

#[test]
fn since_bounds_by_when_a_claim_became_true_not_when_it_was_written() {
    // Consolidation runs behind the work, sometimes by hours, so recorded_at answers "when did
    // the consolidator get round to it" and valid_from answers the question people actually ask.
    let (mut ledger, project, worktree) = fixture();
    let event = append_event(&mut ledger, project, worktree, 1, "a turn");

    let mut old = memory(project, worktree, "Older claim", vec![event]);
    old.valid_from = time::OffsetDateTime::UNIX_EPOCH;
    let mut recent = memory(project, worktree, "Newer claim", vec![event]);
    recent.valid_from = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(100);
    ledger.append_memory(&old).expect("append old");
    ledger.append_memory(&recent).expect("append recent");

    let temp = tempfile::tempdir().expect("export dir");
    let report = export_project(
        &ledger,
        project,
        temp.path(),
        ExportFormat::Jsonl,
        Some(time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(50)),
    )
    .expect("export");

    assert_eq!(report.memories, 1);
    let memories = std::fs::read_to_string(temp.path().join("memories.jsonl")).expect("read");
    assert!(memories.contains("Newer claim"));
    assert!(!memories.contains("Older claim"));
}

#[test]
fn an_empty_project_exports_an_empty_bundle_rather_than_failing() {
    // Exporting is how someone leaves, and refusing to export a project that happens to hold
    // nothing would be the worst possible moment to be pedantic.
    let (ledger, project, _) = fixture();
    let temp = tempfile::tempdir().expect("export dir");
    let report = export_project(&ledger, project, temp.path(), ExportFormat::Both, None)
        .expect("export an empty project");
    assert_eq!(report.memories, 0);
    assert_eq!(report.events, 0);
    assert!(temp.path().join("memories.jsonl").is_file());
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
}

fn append_event(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    offset: i64,
    content: &str,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    ledger
        .append_batch(&EventBatch {
            source_id: "export-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "export-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
                observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "export:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": content }),
                raw: serde_json::json!({ "content": content }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(offset as u64),
        })
        .expect("append event");
    event_id
}

fn memory(
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    evidence_ids: Vec<uuid::Uuid>,
) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("{title}: exported."),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids,
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
