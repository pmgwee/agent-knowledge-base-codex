//! Searching events and memories together.
//!
//! Needs no model: this is a keyword-channel property, and it was broken in a way the vector
//! channel could not have rescued.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn a_matching_memory_is_reachable_even_when_events_outnumber_it() {
    // Measured on the live ledger. Events and memories live in different FTS tables, with
    // different field weights and different corpus statistics, so their BM25 scores are numbers
    // on incompatible scales — the exact thing RRF exists to avoid comparing. They were merged
    // into one list and sorted by raw score, and the larger corpus won every time: the same query
    // scored events up to 16.7 and memories up to 13.2, so with 25,174 events against 2,097
    // memories a mixed search returned events for every slot.
    //
    // Forty-four memories mentioned the query term. None of them was reachable.
    //
    // **This fixture does not reproduce that.** At forty documents the shared term appears in all
    // of them, so its IDF — and therefore every BM25 score — is zero, and the scale gap the bug
    // depends on cannot exist. It is kept as a guard on the property rather than as a reproduction
    // of the defect; the defect itself was measured on the live ledger and re-checked there after
    // the fix, which is the evidence that matters for a corpus-statistics bug.
    let (mut ledger, project, worktree) = fixture();

    let events: Vec<NormalizedEvent> = (0..40)
        .map(|index| {
            event(
                project,
                worktree,
                index,
                "the deployment pipeline ran and the deployment finished",
            )
        })
        .collect();
    append(&mut ledger, events);

    // Cite an event that exists, so the append is accepted on its merits.
    let cited = ledger
        .search(
            &SearchQuery::text(project, "deployment")
                .events_only()
                .with_limit(1),
        )
        .expect("find an event to cite")
        .first()
        .map(|hit| hit.source_id)
        .expect("the fixture has events");

    let record = memory(
        project,
        worktree,
        "Deployment pipeline decision",
        vec![cited],
    );
    ledger.append_memory(&record).expect("append memory");

    let hits = ledger
        .search(&SearchQuery::text(project, "deployment").with_limit(5))
        .expect("mixed search");

    assert!(
        hits.iter().any(|hit| hit.memory_id == Some(record.id)),
        "a matching memory must be reachable when events outnumber it, got sources {:?}",
        hits.iter().map(|hit| hit.source).collect::<Vec<_>>()
    );
}

#[test]
fn a_single_source_filter_still_returns_only_that_source() {
    // Splitting the keyword channel in two must not leak the other one back in.
    let (mut ledger, project, worktree) = fixture();
    append(&mut ledger, vec![event(project, worktree, 1, "deployment")]);
    let cited = ledger
        .search(
            &SearchQuery::text(project, "deployment")
                .events_only()
                .with_limit(1),
        )
        .expect("search")
        .first()
        .map(|hit| hit.source_id)
        .expect("one event");
    let record = memory(project, worktree, "Deployment decision", vec![cited]);
    ledger.append_memory(&record).expect("append");

    let memories = ledger
        .search(&SearchQuery::text(project, "deployment").memories_only())
        .expect("memories only");
    assert!(!memories.is_empty());
    assert!(memories.iter().all(|hit| hit.memory_id.is_some()));

    let events = ledger
        .search(&SearchQuery::text(project, "deployment").events_only())
        .expect("events only");
    assert!(!events.is_empty());
    assert!(events.iter().all(|hit| hit.memory_id.is_none()));
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
}

fn append(ledger: &mut EventLedger, events: Vec<NormalizedEvent>) {
    let count = u64::try_from(events.len()).expect("count");
    ledger
        .append_batch(&EventBatch {
            source_id: "mixed-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(count),
        })
        .expect("append events");
}

fn event(project: ProjectId, worktree: WorktreeId, offset: i64, content: &str) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    key[1] = (offset >> 8) as u8;
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: format!("session-{offset}"),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(offset),
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: offset,
        source_schema: "mixed:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": content }),
        raw: serde_json::json!({ "content": content }),
    }
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
        content: "The deployment pipeline decision and its reasoning.".to_owned(),
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
