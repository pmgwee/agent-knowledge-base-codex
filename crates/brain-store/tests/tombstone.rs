//! Withdrawing a claim, and the paths that must all agree it is gone.
//!
//! The risk this feature carries is not that withdrawal fails loudly — it is that one read path
//! forgets. A memory that disappears from search but survives in the vault, or vanishes from the
//! vault but still reaches an agent's orientation, is worse than no withdrawal at all: the user
//! believes it is gone. So the central test walks every read path rather than one.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn a_withdrawn_memory_disappears_from_every_read_path() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, "regrettable secret plan");
    let doomed = memory(project, worktree, "Regrettable secret plan", vec![evidence]);
    let kept = memory(project, worktree, "Perfectly fine decision", vec![evidence]);
    ledger.append_memory(&doomed).expect("append doomed");
    ledger.append_memory(&kept).expect("append kept");

    // Present on every path before withdrawal, so the assertions after mean something.
    assert_eq!(ledger.current_project_memories().expect("list").len(), 2);
    assert!(ledger.current_memory(doomed.id).expect("lookup").is_some());
    assert!(
        found(&ledger, project, "regrettable"),
        "search must find it first, or the test proves nothing"
    );

    ledger
        .forget_memory(
            doomed.id,
            "captured something I did not mean to keep",
            "quekm",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("withdraw");

    // 1. Listing — the path projection and export both go through.
    let listed = ledger.current_project_memories().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, kept.id);

    // 2. Single lookup — a listing and a lookup must not disagree about existence.
    assert!(ledger.current_memory(doomed.id).expect("lookup").is_none());
    assert!(ledger.current_memory(kept.id).expect("lookup").is_some());

    // 3. Search — the path an agent reaches it by.
    assert!(
        !found(&ledger, project, "regrettable"),
        "a withdrawn memory must not be retrievable"
    );
    assert!(
        found(&ledger, project, "perfectly"),
        "and its neighbours must be unaffected"
    );

    // 4. The evidence is untouched. That is the whole point of a tombstone over a delete: the
    //    ledger stayed append-only and the turn is still there for anything else that cites it.
    assert!(
        ledger.event(evidence).expect("event lookup").is_some(),
        "withdrawal removes a claim, never the evidence under it"
    );
}

#[test]
fn a_withdrawal_records_who_and_why() {
    // Six months later, a tombstone with no reason is indistinguishable from corruption.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, "a turn");
    let record = memory(project, worktree, "Something", vec![evidence]);
    ledger.append_memory(&record).expect("append");

    let tombstone = ledger
        .forget_memory(record.id, "  wrong project  ", "quekm", stamp(42))
        .expect("withdraw");

    assert_eq!(tombstone.memory_id, record.id);
    assert_eq!(tombstone.reason, "wrong project", "reason is trimmed");
    assert_eq!(tombstone.redacted_by, "quekm");
    assert_eq!(tombstone.redacted_at, stamp(42));

    assert_eq!(ledger.tombstones().expect("list").len(), 1);
}

#[test]
fn withdrawing_twice_is_not_an_error_but_withdrawing_nothing_is() {
    // Idempotent, because a retried command should not fail. Not idempotent about *absence*,
    // because a mistyped id silently succeeding would tell someone they had removed something
    // they had not.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, "a turn");
    let record = memory(project, worktree, "Something", vec![evidence]);
    ledger.append_memory(&record).expect("append");

    let first = ledger
        .forget_memory(record.id, "first", "quekm", stamp(1))
        .expect("first withdrawal");
    let second = ledger
        .forget_memory(record.id, "second", "quekm", stamp(2))
        .expect("second withdrawal is not an error");
    assert_eq!(
        second.reason, first.reason,
        "the original withdrawal stands; a repeat does not rewrite the record"
    );
    assert_eq!(ledger.tombstones().expect("list").len(), 1);

    let error = ledger
        .forget_memory(uuid::Uuid::now_v7(), "typo", "quekm", stamp(3))
        .expect_err("withdrawing an absent memory must fail");
    assert!(format!("{error}").contains("no memory"), "{error}");
}

#[test]
fn a_withdrawal_needs_a_reason() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree, "a turn");
    let record = memory(project, worktree, "Something", vec![evidence]);
    ledger.append_memory(&record).expect("append");

    let error = ledger
        .forget_memory(record.id, "   ", "quekm", stamp(1))
        .expect_err("an empty reason must be refused");
    assert!(format!("{error}").contains("reason"), "{error}");
}

// --- fixtures ---

fn stamp(seconds: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds)
}

fn found(ledger: &EventLedger, project: ProjectId, text: &str) -> bool {
    !ledger
        .search(&SearchQuery::text(project, text).memories_only())
        .expect("search")
        .is_empty()
}

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
    content: &str,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "tombstone-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "tombstone-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: 1,
                source_schema: "tombstone:v1".to_owned(),
                raw_hash: [7; 32],
                idempotency_key: [7; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": content }),
                raw: serde_json::json!({ "content": content }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
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
        content: format!("{title}. Recorded for the tombstone test."),
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
