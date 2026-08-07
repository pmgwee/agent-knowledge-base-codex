//! Hybrid retrieval end to end, against the real model.
//!
//! The unit tests in `search.rs` pin the fusion arithmetic with hand-built rankings. This pins
//! the thing that actually has to be true: that a question whose evidence shares no vocabulary
//! with it becomes findable, and that turning the vector channel on does not cost anything the
//! keyword channel was already delivering.
//!
//! Ignored by default because it needs the 87 MB checkpoint. Fetch it with:
//!
//! ```text
//! MODELS=~/AgentBrain/models/all-MiniLM-L6-v2
//! mkdir -p "$MODELS"
//! BASE=https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main
//! for f in config.json tokenizer.json model.safetensors; do curl -sL -o "$MODELS/$f" "$BASE/$f"; done
//! cargo test -p brain-store --test hybrid_search -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery, shared_embedder};

/// The shape of a `single-session-preference` question: the answer is stated in words the
/// question never uses. This is the 63.3% category, and the reason embeddings exist here.
const QUESTION: &str = "what do I usually prefer when deploying";
const ANSWER: &str = "I always ship straight to production on Fridays, never through staging";

/// Ordinary ledger content from other sessions. Each shares more of the question's *words* than
/// the answer does — which is what makes keyword search rank them first — while being about
/// something else entirely.
const HAYSTACK: [&str; 3] = [
    "what do we do when the integration tests fail on a Friday afternoon",
    "the backup retention policy keeps 24 hourly and 30 daily snapshots",
    "I usually check the logs first before restarting anything",
];

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn a_vector_reaches_evidence_that_shares_no_words_with_the_question() {
    // Measured similarities against this question, so the margin is on the record rather than
    // assumed: answer 0.353, and the three distractors 0.145, 0.005, -0.010. Against the
    // question's *words*, the ordering is the other way round — the first distractor carries
    // three query terms and the answer carries one.
    //
    // The margin is real but narrower than it looks, and the limit is worth stating here rather
    // than discovering later. `all-MiniLM-L6-v2` is a bi-encoder: it scores how alike two texts
    // are, not whether one answers the other. Put a turn about deployment *speed* in this
    // haystack and it scores 0.478, comfortably above the turn that actually answers. So this
    // channel buys reach into evidence that shares no vocabulary; it does not buy a judgement
    // about which of several on-topic turns is responsive. That judgement is what a cross-encoder
    // reranker does, and it is the honest next step rather than a weight to be tuned.
    let (mut ledger, project, worktree) = fixture();
    let mut events: Vec<NormalizedEvent> = HAYSTACK
        .iter()
        .enumerate()
        .map(|(index, text)| event(project, worktree, index, &format!("noise-{index}"), text))
        .collect();
    events.push(event(
        project,
        worktree,
        HAYSTACK.len(),
        "the-answer",
        ANSWER,
    ));
    append(&mut ledger, events);

    let query = SearchQuery::text(project, QUESTION)
        .events_only()
        .with_limit(4);

    let keyword = ledger.search(&query).expect("keyword search");
    let keyword_rank = rank_of(&keyword, "the-answer");
    println!("  keyword rank : {keyword_rank:?}");
    assert_ne!(
        keyword_rank,
        Some(0),
        "if keyword already ranks this first there is nothing for fusion to demonstrate"
    );

    embed_all(&mut ledger);
    assert!(
        ledger.enable_vector_search(&brain_home()),
        "model must load for this test to mean anything"
    );

    let hybrid = ledger.search(&query).expect("hybrid search");
    let hybrid_rank = rank_of(&hybrid, "the-answer").expect("the answer must be retrievable");
    println!("  hybrid rank  : {hybrid_rank}");

    assert_eq!(
        hybrid_rank, 0,
        "the turn that answers the question belongs first once meaning is fused in"
    );
}

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn the_vector_channel_still_obeys_the_caller_filters() {
    // A channel that ignored the caller's filters would return rows they explicitly excluded —
    // and because it only fires when a model is installed, it would do so on some machines and
    // not others.
    let (mut ledger, project, worktree) = fixture();
    append(
        &mut ledger,
        vec![
            event(project, worktree, 0, "wanted", ANSWER),
            event(project, worktree, 1, "excluded", ANSWER),
        ],
    );
    embed_all(&mut ledger);
    assert!(ledger.enable_vector_search(&brain_home()));

    let hits = ledger
        .search(
            &SearchQuery::text(project, QUESTION)
                .events_only()
                .for_session("wanted")
                .with_limit(10),
        )
        .expect("filtered hybrid search");

    assert!(!hits.is_empty(), "the wanted session must still be found");
    assert!(
        hits.iter()
            .all(|hit| hit.native_session_id.as_deref() == Some("wanted")),
        "vector results must respect the session filter, got {:?}",
        hits.iter()
            .map(|hit| hit.native_session_id.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn an_exact_keyword_match_is_not_displaced_by_fusion() {
    // The failure mode that would make this feature a net loss: weighting vector above keyword
    // costs nothing on questions keyword answers badly, and must cost nothing on the ones it
    // answers perfectly either.
    let (mut ledger, project, worktree) = fixture();
    let mut events: Vec<NormalizedEvent> = (0..8)
        .map(|index| {
            event(
                project,
                worktree,
                index,
                "noise",
                "unrelated chatter about lunch and the weather outside",
            )
        })
        .collect();
    events.push(event(
        project,
        worktree,
        8,
        "exact",
        "the sentinel token PREFS_ENABLED was flipped on in configuration",
    ));
    append(&mut ledger, events);
    embed_all(&mut ledger);
    assert!(ledger.enable_vector_search(&brain_home()));

    let hits = ledger
        .search(
            &SearchQuery::text(project, "PREFS_ENABLED")
                .events_only()
                .with_limit(5),
        )
        .expect("hybrid search");

    assert_eq!(
        rank_of(&hits, "exact"),
        Some(0),
        "a rare exact token must still rank first once vectors are fused in"
    );
}

// --- fixtures ---

fn brain_home() -> PathBuf {
    std::env::var("BRAIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();
            PathBuf::from(home).join("AgentBrain")
        })
}

fn embed_all(ledger: &mut EventLedger) {
    let home = brain_home();
    let Some(embedder) = shared_embedder(&home) else {
        panic!(
            "model not installed under {}; see the module comment",
            home.display()
        );
    };
    let now = time::OffsetDateTime::UNIX_EPOCH;
    loop {
        let pending = ledger
            .events_awaiting_embedding(32)
            .expect("pending embeddings");
        if pending.is_empty() {
            return;
        }
        for item in pending {
            let vector = if EventLedger::is_embeddable_event_text(&item.text) {
                Some(embedder.embed(&item.text).expect("embed"))
            } else {
                None
            };
            ledger
                .store_event_embedding(item.event_id, vector.as_deref(), now)
                .expect("store");
        }
    }
}

fn rank_of(hits: &[brain_store::SearchHit], session: &str) -> Option<usize> {
    hits.iter()
        .position(|hit| hit.native_session_id.as_deref() == Some(session))
}

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open ledger");
    (ledger, project, worktree)
}

fn append(ledger: &mut EventLedger, events: Vec<NormalizedEvent>) {
    let count = u64::try_from(events.len()).expect("event count");
    ledger
        .append_batch(&EventBatch {
            source_id: "hybrid-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(count),
        })
        .expect("append events");
}

fn event(
    project: ProjectId,
    worktree: WorktreeId,
    offset: usize,
    session: &str,
    content: &str,
) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = u8::try_from(offset).unwrap_or(0);
    key[1] = session.len() as u8;
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: session.to_owned(),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "hybrid.jsonl".to_owned(),
        source_offset: i64::try_from(offset).unwrap_or(0),
        source_schema: "hybrid:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": content }),
        raw: serde_json::json!({ "content": content }),
    }
}
