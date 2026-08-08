//! Counting what retrieval reaches.
//!
//! Decay needs an input and the only honest one is use. These pin that the counter measures what
//! it claims to, and that a search never fails to protect a statistic.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn retrieval_counts_every_memory_it_returned_not_one_per_search() {
    // Counting once per search rather than once per result would make a memory that always appears
    // eighth look exactly as used as one that never appears at all.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree);
    let first = memory(
        project,
        worktree,
        "Deployment pipeline note",
        vec![evidence],
    );
    let second = memory(
        project,
        worktree,
        "Deployment rollback note",
        vec![evidence],
    );
    ledger.append_memory(&first).expect("append first");
    ledger.append_memory(&second).expect("append second");

    assert_eq!(
        ledger.never_retrieved_memory_count().expect("never"),
        2,
        "nothing has been retrieved yet"
    );

    let hits = ledger
        .search(&SearchQuery::text(project, "deployment").memories_only())
        .expect("search");
    assert_eq!(hits.len(), 2, "both memories match");

    assert_eq!(ledger.never_retrieved_memory_count().expect("never"), 0);
    for record in [&first, &second] {
        let access = ledger
            .memory_access(record.id)
            .expect("access")
            .expect("a returned memory is counted");
        assert_eq!(access.retrieved_count, 1);
    }
}

#[test]
fn repeated_retrieval_accumulates() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree);
    let record = memory(
        project,
        worktree,
        "Deployment pipeline note",
        vec![evidence],
    );
    ledger.append_memory(&record).expect("append");

    for _ in 0..3 {
        // A fresh query each time: an identical one is served from the search cache, which is
        // correct behaviour and would count once rather than three times.
        let text = format!("deployment {}", uuid::Uuid::now_v7());
        let _ = ledger.search(&SearchQuery::text(project, text).memories_only());
    }

    let access = ledger
        .memory_access(record.id)
        .expect("access")
        .expect("counted");
    assert_eq!(access.retrieved_count, 3);
}

#[test]
fn a_memory_nothing_asked_for_stays_uncounted() {
    // The signal only means something if silence is recorded as silence.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree);
    let wanted = memory(
        project,
        worktree,
        "Deployment pipeline note",
        vec![evidence],
    );
    let ignored = memory(project, worktree, "Unrelated backup policy", vec![evidence]);
    ledger.append_memory(&wanted).expect("append");
    ledger.append_memory(&ignored).expect("append");

    let _ = ledger.search(&SearchQuery::text(project, "deployment").memories_only());

    assert!(ledger.memory_access(wanted.id).expect("access").is_some());
    assert!(
        ledger.memory_access(ignored.id).expect("access").is_none(),
        "a memory nothing asked for must not be counted as used"
    );
    assert_eq!(ledger.never_retrieved_memory_count().expect("never"), 1);
}

#[test]
fn staleness_needs_both_age_and_disuse_and_reverses_on_retrieval() {
    // Neither condition means anything alone: a decision from March can be perfectly current, and
    // a memory recorded yesterday has had no chance to be wanted. And there is no flag to clear —
    // staleness is computed from two facts that both move on their own, so retrieval undoes it.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_event(&mut ledger, project, worktree);
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(500);
    let quiet_for = time::Duration::days(90);

    let mut old = memory(project, worktree, "Old and unwanted", vec![evidence]);
    old.valid_from = now - time::Duration::days(200);
    let mut recent = memory(project, worktree, "Recent and unwanted", vec![evidence]);
    recent.valid_from = now - time::Duration::days(10);
    ledger.append_memory(&old).expect("append old");
    ledger.append_memory(&recent).expect("append recent");

    let stale = ledger.stale_memory_ids(now, quiet_for).expect("stale");
    assert!(stale.contains(&old.id), "old and unretrieved is stale");
    assert!(
        !stale.contains(&recent.id),
        "recent is not stale however unused — it has had no chance to be wanted"
    );

    // Retrieval reverses it, with nothing to clear.
    ledger
        .record_memory_access(&[old.id], now)
        .expect("record access");
    assert!(
        !ledger
            .stale_memory_ids(now, quiet_for)
            .expect("stale")
            .contains(&old.id),
        "a memory that was just asked for is not stale"
    );
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
}

fn append_event(ledger: &mut EventLedger, project: ProjectId, worktree: WorktreeId) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "access-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "access-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: 1,
                source_schema: "access:v1".to_owned(),
                raw_hash: [5; 32],
                idempotency_key: [5; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": "a turn" }),
                raw: serde_json::json!({ "content": "a turn" }),
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
        content: format!("{title}."),
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
