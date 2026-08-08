//! A subject page's paragraph is only ever shown for the set it was written about.
//!
//! This is the property that makes prose safe on a page whose entire design was "asserts nothing".

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn a_paragraph_is_returned_for_the_set_it_was_written_about() {
    let (mut ledger, project, worktree) = fixture();
    let ids = vec![
        memory(&mut ledger, project, worktree, "Deploy on commit"),
        memory(&mut ledger, project, worktree, "Build gate"),
    ];
    ledger
        .store_subject_synthesis(
            "deployment",
            &ids,
            "Deployment happens on commit. ([[Deploy on commit]])",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("store");

    let stored = ledger
        .subject_synthesis("deployment", &ids)
        .expect("lookup")
        .expect("a paragraph for the same set");
    assert!(stored.markdown.contains("Deployment happens on commit"));
    assert_eq!(ledger.subject_synthesis_count().expect("count"), 1);
}

#[test]
fn a_subject_that_gained_a_memory_has_no_paragraph_rather_than_a_stale_one() {
    // The whole design. Prose describing two memories is wrong once there are three, and a subject
    // page gains memories continuously — that is what it is for. Returning the old text would let
    // the page assert something the ledger no longer supports, and there is no length of window in
    // which that is acceptable.
    let (mut ledger, project, worktree) = fixture();
    let mut ids = vec![
        memory(&mut ledger, project, worktree, "Deploy on commit"),
        memory(&mut ledger, project, worktree, "Build gate"),
    ];
    ledger
        .store_subject_synthesis(
            "deployment",
            &ids,
            "Two things are true about deployment.",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("store");

    ids.push(memory(&mut ledger, project, worktree, "Rollback procedure"));

    assert!(
        ledger
            .subject_synthesis("deployment", &ids)
            .expect("lookup")
            .is_none(),
        "a changed subject must have no paragraph, not an old one"
    );
}

#[test]
fn reordering_the_same_memories_keeps_the_paragraph() {
    // Ranking order is not part of what the prose describes. If it were, every shift in retrieval
    // would throw away perfectly good synthesis and pay to regenerate it.
    let (mut ledger, project, worktree) = fixture();
    let ids = vec![
        memory(&mut ledger, project, worktree, "Deploy on commit"),
        memory(&mut ledger, project, worktree, "Build gate"),
    ];
    ledger
        .store_subject_synthesis(
            "deployment",
            &ids,
            "A paragraph.",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("store");

    let reversed: Vec<uuid::Uuid> = ids.iter().rev().copied().collect();
    assert!(
        ledger
            .subject_synthesis("deployment", &reversed)
            .expect("lookup")
            .is_some()
    );
}

#[test]
fn regenerating_replaces_rather_than_accumulates() {
    let (mut ledger, project, worktree) = fixture();
    let ids = vec![memory(&mut ledger, project, worktree, "Deploy on commit")];
    for text in ["First attempt.", "Second attempt."] {
        ledger
            .store_subject_synthesis("deployment", &ids, text, time::OffsetDateTime::UNIX_EPOCH)
            .expect("store");
    }
    assert_eq!(ledger.subject_synthesis_count().expect("count"), 1);
    assert_eq!(
        ledger
            .subject_synthesis("deployment", &ids)
            .expect("lookup")
            .expect("stored")
            .markdown,
        "Second attempt."
    );
}

#[test]
fn one_subject_s_paragraph_is_not_returned_for_another() {
    let (mut ledger, project, worktree) = fixture();
    let ids = vec![memory(&mut ledger, project, worktree, "Deploy on commit")];
    ledger
        .store_subject_synthesis(
            "deployment",
            &ids,
            "About deployment.",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("store");
    assert!(
        ledger
            .subject_synthesis("retrieval", &ids)
            .expect("lookup")
            .is_none()
    );
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    ledger
        .append_batch(&EventBatch {
            source_id: "synthesis-fixture".to_owned(),
            events: vec![event(project, worktree)],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
    (ledger, project, worktree)
}

fn memory(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
) -> uuid::Uuid {
    let cited = ledger
        .search(
            &SearchQuery::text(project, "deployment")
                .events_only()
                .with_limit(1),
        )
        .expect("find event")
        .first()
        .map(|hit| hit.source_id)
        .expect("one event");
    let record = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("{title} and why."),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids: vec![cited],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    };
    let id = record.id;
    ledger.append_memory(&record).expect("append memory");
    id
}

fn event(project: ProjectId, worktree: WorktreeId) -> NormalizedEvent {
    let key = [7_u8; 32];
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "session-1".to_owned(),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: 0,
        source_schema: "synthesis:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": "the deployment pipeline ran" }),
        raw: serde_json::json!({ "content": "the deployment pipeline ran" }),
    }
}
