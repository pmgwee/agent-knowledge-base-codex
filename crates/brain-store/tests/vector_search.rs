//! Vector storage and search over memories.
//!
//! The similarity behaviour is exercised with hand-written vectors rather than the model, so
//! these run everywhere and fail for one reason only. Whether the real model produces sensible
//! geometry is a separate question, answered by `embedding_model.rs`.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EMBEDDING_DIMENSIONS, EventLedger, PendingEmbedding};

#[test]
fn a_stored_vector_is_found_and_ranked_by_similarity() {
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_evidence(&mut ledger, project, worktree);

    let near = memory(project, worktree, "Deploy on commit", vec![evidence]);
    let far = memory(project, worktree, "Backup retention policy", vec![evidence]);
    ledger.append_memory(&near).expect("append near");
    ledger.append_memory(&far).expect("append far");

    // Two orthogonal directions, and a query sitting close to the first.
    let mut near_vec = unit(0);
    let far_vec = unit(1);
    let query = unit(0);
    // Nudge so the match is close but not identical, as a real one would be.
    near_vec[2] = 0.05;
    renormalise(&mut near_vec);

    store(&mut ledger, &near, &near_vec);
    store(&mut ledger, &far, &far_vec);

    let hits = ledger.search_by_vector(&query, 10).expect("vector search");
    assert_eq!(hits.len(), 2, "both memories are current and embedded");
    assert_eq!(
        hits[0].memory_id, near.id,
        "the nearer memory must rank first"
    );
    assert!(hits[0].similarity > 0.9, "got {}", hits[0].similarity);
    assert!(hits[1].similarity.abs() < 0.1, "got {}", hits[1].similarity);
}

#[test]
fn coverage_reports_what_is_still_unembedded() {
    // A backfill needs to know what is left, and a half-finished index must be visible rather
    // than looking like a complete one that simply misses things.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_evidence(&mut ledger, project, worktree);
    let one = memory(project, worktree, "First", vec![evidence]);
    let two = memory(project, worktree, "Second", vec![evidence]);
    ledger.append_memory(&one).expect("append one");
    ledger.append_memory(&two).expect("append two");

    assert_eq!(ledger.embedding_coverage().expect("coverage"), (0, 2));
    assert_eq!(
        ledger
            .memories_awaiting_embedding(10)
            .expect("pending")
            .len(),
        2
    );

    store(&mut ledger, &one, &unit(0));
    assert_eq!(ledger.embedding_coverage().expect("coverage"), (1, 1));

    let pending = ledger.memories_awaiting_embedding(10).expect("pending");
    assert_eq!(pending.len(), 1, "an embedded memory is not offered again");
    assert_eq!(pending[0].memory_id, two.id);
}

#[test]
fn the_text_offered_for_embedding_carries_title_and_content() {
    // A query may resemble either the claim or its detail. Embedding only one of them makes
    // half of every memory unreachable by meaning.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_evidence(&mut ledger, project, worktree);
    let record = memory(project, worktree, "Bound the job window", vec![evidence]);
    ledger.append_memory(&record).expect("append");

    let pending = ledger.memories_awaiting_embedding(1).expect("pending");
    assert_eq!(pending.len(), 1);
    assert!(pending[0].text.contains("Bound the job window"), "title");
    assert!(pending[0].text.contains("Bound the job window."), "content");
}

#[test]
fn a_vector_of_the_wrong_width_is_refused_rather_than_stored() {
    // A short vector would silently compare against the wrong coordinates forever.
    let (mut ledger, project, worktree) = fixture();
    let evidence = append_evidence(&mut ledger, project, worktree);
    let record = memory(project, worktree, "Anything", vec![evidence]);
    ledger.append_memory(&record).expect("append");
    let pending = ledger
        .memories_awaiting_embedding(1)
        .expect("pending")
        .remove(0);

    let error = ledger
        .store_memory_embedding(&pending, &[0.0; 8], time::OffsetDateTime::UNIX_EPOCH)
        .expect_err("a 8-dimensional vector must be refused");
    assert!(format!("{error}").contains("dimensional"), "{error}");
}

#[test]
fn searching_with_no_embeddings_returns_nothing_rather_than_failing() {
    // Every brain starts here, and one that never installs the model stays here. Keyword search
    // must be unaffected.
    let (ledger, _, _) = fixture();
    assert!(
        ledger
            .search_by_vector(&unit(0), 5)
            .expect("search an empty index")
            .is_empty()
    );
}

// --- fixtures ---

fn unit(axis: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    v[axis] = 1.0;
    v
}

fn renormalise(v: &mut [f32]) {
    let length: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for value in v.iter_mut() {
        *value /= length;
    }
}

fn store(ledger: &mut EventLedger, record: &MemoryRecord, vector: &[f32]) {
    let pending = PendingEmbedding {
        version_id: record.version_id,
        memory_id: record.id,
        text: record.title.clone(),
    };
    ledger
        .store_memory_embedding(&pending, vector, time::OffsetDateTime::UNIX_EPOCH)
        .expect("store embedding");
}

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
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

fn append_evidence(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "fixture-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture.jsonl".to_owned(),
                source_offset: 1,
                source_schema: "fixture:v1".to_owned(),
                raw_hash: [1; 32],
                idempotency_key: [2; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": "fixture"}),
                raw: serde_json::json!({"content": "fixture"}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append evidence");
    event_id
}
