//! Re-ranking must obey the optional-retrieval invariant: it may reorder, never lose.
//!
//! The first test needs no model — it is the case that must hold on every machine, including the
//! ones that will never install a checkpoint. The second needs the model and pins the slice
//! discipline that keeps two incomparable score systems out of one sorted list.

use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn a_brain_with_no_checkpoint_searches_exactly_as_it_did_before() {
    // `enable_reranking` on a home with no model must report false and change nothing. A stage that
    // half-enabled itself would make retrieval differ between machines for no visible reason.
    let (mut ledger, project, worktree) = fixture(12);
    let query = SearchQuery::text(project, "deployment pipeline").with_limit(10);
    let before = ledger.search(&query).expect("search");

    let empty = tempfile::tempdir().expect("temp");
    assert!(
        !ledger.enable_reranking(empty.path()),
        "no checkpoint means no re-ranking"
    );
    assert!(!ledger.reranking_enabled());

    let after = ledger.search(&query).expect("search again");
    assert_eq!(
        before
            .iter()
            .map(|hit| hit.source_id)
            .collect::<Vec<uuid::Uuid>>(),
        after
            .iter()
            .map(|hit| hit.source_id)
            .collect::<Vec<uuid::Uuid>>(),
        "the order must be untouched when the stage is unavailable"
    );
    assert!(
        after.iter().all(|hit| hit.rerank_score.is_none()),
        "nothing may carry a re-rank score when nothing re-ranked"
    );
    let _ = worktree;
}

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see reranker_model.rs"]
fn re_ranking_reorders_the_head_and_returns_the_same_documents() {
    // Recall is the invariant. Re-ranking is allowed to change *which order* the answer arrives in
    // and never *whether* it arrives — so the returned set must match to the document.
    let (mut ledger, project, _worktree) = fixture(40);
    let query = SearchQuery::text(project, "deployment pipeline")
        .without_session_diversity()
        .with_limit(30);
    let before = ledger.search(&query).expect("search");

    let home = brain_home();
    if !ledger.enable_reranking(&home) {
        panic!("model not installed under {}", home.display());
    }
    let after = ledger.search(&query).expect("re-ranked search");

    let mut before_ids: Vec<uuid::Uuid> = before.iter().map(|hit| hit.source_id).collect();
    let mut after_ids: Vec<uuid::Uuid> = after.iter().map(|hit| hit.source_id).collect();
    assert_eq!(
        before_ids.len(),
        after_ids.len(),
        "re-ranking must not change how many results come back"
    );
    before_ids.sort();
    after_ids.sort();
    assert_eq!(
        before_ids, after_ids,
        "re-ranking may reorder, never substitute"
    );

    // Only the head carries a score, and the tail keeps the fused order it arrived with.
    let scored = after
        .iter()
        .filter(|hit| hit.rerank_score.is_some())
        .count();
    assert!(
        scored > 0 && scored <= 16,
        "exactly the window should be scored, got {scored}"
    );
    assert!(
        after
            .iter()
            .skip_while(|hit| hit.rerank_score.is_some())
            .all(|hit| hit.rerank_score.is_none()),
        "scored hits must form a prefix — a scored hit below an unscored one means the two score \
         systems were sorted together"
    );
}

// --- fixtures ---

fn brain_home() -> std::path::PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    std::path::PathBuf::from(home).join("AgentBrain")
}

fn fixture(count: i64) -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let events: Vec<NormalizedEvent> = (0..count)
        .map(|index| {
            event(
                project,
                worktree,
                index,
                &format!(
                    "the deployment pipeline step {index} ran and the deployment finished cleanly"
                ),
            )
        })
        .collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "rerank-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(u64::try_from(count).expect("count")),
        })
        .expect("append");
    (ledger, project, worktree)
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
        source_schema: "rerank:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": content }),
        raw: serde_json::json!({ "content": content }),
    }
}
