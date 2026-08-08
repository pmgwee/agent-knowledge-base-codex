//! Closing the vocabulary gap with the corpus's own words.
//!
//! The measured problem: `ship to production` and `deploy` are the same claim and score ten points
//! apart. Neither the bi-encoder nor the cross-encoder can invent a link between words the corpus
//! keeps separate — and neither has to. The corpus contains both. Documents about shipping are
//! written using the word *deploy*, so asking twice, the second time in the corpus's vocabulary,
//! reaches what one pass could not.
//!
//! The property that matters more than the gain: expansion is fused as an extra channel, so it
//! **cannot lose** a result the plain query found. An added stage may only add.

use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery};

#[test]
fn expansion_never_loses_what_the_plain_query_found() {
    // The invariant, and the reason expansion is a channel rather than a replacement. A rewritten
    // query can drop the very document that seeded it; a fused one cannot.
    let (ledger, project) = fixture();
    let plain: Vec<uuid::Uuid> = ledger
        .search(&SearchQuery::text(project, "deployment pipeline").with_limit(20))
        .expect("plain")
        .into_iter()
        .map(|hit| hit.source_id)
        .collect();
    let expanded: Vec<uuid::Uuid> = ledger
        .search(
            &SearchQuery::text(project, "deployment pipeline")
                .with_expansion()
                .with_limit(20),
        )
        .expect("expanded")
        .into_iter()
        .map(|hit| hit.source_id)
        .collect();

    for id in &plain {
        assert!(
            expanded.contains(id),
            "expansion dropped {id}, which the plain query returned"
        );
    }
}

#[test]
fn expansion_reaches_a_document_that_shares_no_word_with_the_question() {
    // The gap itself. "shipping" never appears in the target document — it says "deploy" — and no
    // amount of re-scoring bridges that. The bridge is a third document that uses both words.
    let (ledger, project) = fixture();
    // Two words, neither of which appears in the target document. "how do we handle shipping"
    // does not work as a probe: FTS terms are OR-joined, so "we" alone matches the target and the
    // plain query looks like it already succeeded.
    let query = SearchQuery::text(project, "shipping approach").with_limit(10);

    let plain = ledger.search(&query).expect("plain");
    let expanded = ledger
        .search(&query.clone().with_expansion())
        .expect("expanded");

    // Look for the *target*, not for the word. The bridging documents contain "deploy" as well —
    // checking for the word alone passes on the bridges and proves nothing.
    let found_target = |hits: &[brain_store::SearchHit]| {
        hits.iter()
            .any(|hit| hit.text.to_lowercase().contains("fridays"))
    };

    assert!(
        !found_target(&plain),
        "the fixture is wrong if the plain query already reached the target"
    );
    assert!(
        found_target(&expanded),
        "expansion should reach the target through the bridging documents"
    );
}

#[test]
fn expansion_is_off_unless_asked_for() {
    let (ledger, project) = fixture();
    let query = SearchQuery::text(project, "deployment pipeline");
    assert!(!query.expand_query, "off by default");
    // And an unexpanded search is byte-identical to what it was before the feature existed.
    let a = ledger.search(&query).expect("a");
    let b = ledger.search(&query).expect("b");
    assert_eq!(
        a.iter().map(|h| h.source_id).collect::<Vec<_>>(),
        b.iter().map(|h| h.source_id).collect::<Vec<_>>()
    );
}

#[test]
fn a_query_with_nothing_to_add_expands_to_nothing() {
    // Silence is the common correct answer. An empty corpus has no vocabulary to borrow.
    let project = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open");
    let hits = ledger
        .search(&SearchQuery::text(project, "anything at all").with_expansion())
        .expect("search");
    assert!(hits.is_empty());
}

// --- fixtures ---

/// A corpus with the gap in it: one document says "shipping", one says "deploy", and a third uses
/// both — which is what makes the bridge possible without a model.
fn fixture() -> (EventLedger, ProjectId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let texts = [
        "the deployment pipeline runs on every commit and the build must pass",
        "deployment pipeline configuration lives in the repository root",
        // Two bridges: each contains the question's word *and* the target's. Two rather than one
        // because a term appearing in a single seed document is about that document, not about the
        // subject, and the harvester filters those out — correctly.
        "our shipping process means we deploy to production",
        "shipping is how the team refers to deploy in conversation",
        // The target: says deploy, never says shipping.
        "deploy straight to production on Fridays and never through staging",
        "unrelated chatter about lunch and the weather outside today",
        "the backup retention policy keeps twenty four hourly snapshots",
    ];
    let events: Vec<NormalizedEvent> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| event(project, worktree, index as i64, text))
        .collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "expansion-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(texts.len() as u64),
        })
        .expect("append");
    (ledger, project)
}

fn event(project: ProjectId, worktree: WorktreeId, offset: i64, content: &str) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    key[1] = 91;
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
        source_schema: "expansion:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": content }),
        raw: serde_json::json!({ "content": content }),
    }
}
