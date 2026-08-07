//! Walking a claim back to its evidence.
//!
//! The citations were always in the data; what was missing was any way to exercise them. A
//! citation nobody can follow is indistinguishable from a citation nobody checked.

use brain_cli::verify_memory;
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_claim_walks_back_to_the_turns_it_came_from() {
    let (mut ledger, project, worktree) = fixture();
    let first = append_event(
        &mut ledger,
        project,
        worktree,
        1,
        "chose jose over jsonwebtoken",
    );
    let second = append_event(
        &mut ledger,
        project,
        worktree,
        2,
        "because Edge has no crypto",
    );

    let record = memory(project, worktree, "JWT library choice", vec![first, second]);
    ledger.append_memory(&record).expect("append memory");

    let report = verify_memory(&ledger, project, record.id).expect("walk provenance");

    assert!(report.intact(), "every citation must resolve");
    assert_eq!(report.evidence.len(), 2);
    assert_eq!(report.title, "JWT library choice");
    assert_eq!(report.version_count, 1);

    // Oldest first, so the list reads in the order the work happened.
    assert_eq!(report.evidence[0].event_id, first);
    assert_eq!(report.evidence[1].event_id, second);

    // The chain has to end somewhere this brain does not control: a file anyone can open.
    assert_eq!(report.evidence[0].source_locator, "transcript.jsonl");
    assert_eq!(report.evidence[0].source_offset, 1);
    assert!(
        report.evidence[0].excerpt.contains("jose"),
        "the excerpt must make the turn recognisable, got {:?}",
        report.evidence[0].excerpt
    );

    let rendered = brain_cli::render_provenance(&report);
    assert!(rendered.contains(&format!("memory:{}", record.id)));
    assert!(rendered.contains(&format!("event:{first}")));
    assert!(rendered.contains("transcript.jsonl:1"));
}

#[test]
fn a_memory_citing_an_event_the_ledger_does_not_hold_is_never_stored() {
    // Written expecting to exercise the unresolved-citation path, and it could not: the ledger
    // refuses the append outright. That is a stronger guarantee than the report assumed, and it
    // is the reason `verify` can promise that every citation resolves rather than merely
    // reporting how many did.
    //
    // The unresolved branch stays in the report as defence in depth — it covers corruption
    // arriving some way other than an append — but it is unreachable through the supported path,
    // and this test is what pins that.
    let (mut ledger, project, worktree) = fixture();
    let real = append_event(&mut ledger, project, worktree, 1, "a turn that exists");
    let invented = uuid::Uuid::now_v7();

    let record = memory(project, worktree, "Partly grounded", vec![real, invented]);
    let error = ledger
        .append_memory(&record)
        .expect_err("a memory citing an unknown event must be refused");
    assert!(
        format!("{error:#}").contains(&invented.to_string()),
        "the refusal must name the citation it rejected, got: {error:#}"
    );

    // And nothing was half-written: the memory does not exist at all.
    assert!(
        verify_memory(&ledger, project, record.id).is_err(),
        "a refused append must leave no memory behind"
    );
}

#[test]
fn an_unknown_memory_is_an_error_not_an_empty_report() {
    // An empty report for a memory that does not exist reads as "this claim has no evidence",
    // which is the most damaging possible misreading of this command.
    let (ledger, project, _) = fixture();
    let error = verify_memory(&ledger, project, uuid::Uuid::now_v7())
        .expect_err("an absent memory must not produce a report");
    assert!(format!("{error}").contains("no memory"), "{error}");
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
            source_id: "provenance-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "provenance-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
                observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "provenance:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: Some("main".to_owned()),
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
        content: format!("{title}: recorded for provenance."),
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
