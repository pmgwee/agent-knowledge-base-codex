//! The scheduled reflection.
//!
//! Four scheduled agents is the competitor pattern; the claim attached to it is that the knowledge
//! base "maintains itself". The half we were missing was never the schedule — consolidation already
//! runs continuously — it was that nothing ever stepped back and asked whether what had been built
//! was still coherent. These tests pin the two properties that make the answer trustworthy: it is
//! derived, so a rate-limited provider cannot silence it, and it reports rather than instructs.

use brain_domain::{
    EventBatch, EventType, Harness, MemoryKind, NormalizedEvent, ProjectId, SourceCursor,
    WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_digest_needs_no_provider() {
    // The property worth a test: every number is arithmetic over the ledger. Nothing here can be
    // rate-limited, so the health reading survives exactly the outage that makes it most useful.
    let (ledger, project) = fixture();
    let digest = brain_cli::build_digest(&ledger, project, now()).expect("digest");
    assert_eq!(digest.events, 3);
    assert_eq!(digest.memories, 2);
    assert!(digest.mean_retention > 0.0 && digest.mean_retention <= 1.0);
}

#[test]
fn an_untouched_corpus_reports_that_retrieval_never_reached_it() {
    let (ledger, project) = fixture();
    let digest = brain_cli::build_digest(&ledger, project, now()).expect("digest");
    assert_eq!(digest.never_retrieved, digest.memories);
    assert!(
        digest
            .notes
            .iter()
            .any(|note| note.contains("never reached")),
        "a corpus nothing retrieves is the finding, not a silence: {:?}",
        digest.notes
    );
}

#[test]
fn a_healthy_brain_says_so_rather_than_inventing_a_finding() {
    // The failure mode of every health report: manufacturing concern to justify its own existence.
    let project = ProjectId(uuid::Uuid::now_v7());
    let ledger = EventLedger::open_in_memory(project).expect("open");
    let digest = brain_cli::build_digest(&ledger, project, now()).expect("digest");
    assert_eq!(digest.notes, vec!["Nothing needs a decision.".to_owned()]);
}

#[test]
fn the_markdown_is_shaped_for_the_vault_it_is_appended_to() {
    let (ledger, project) = fixture();
    let digest = brain_cli::build_digest(&ledger, project, now()).expect("digest");
    let markdown = brain_cli::render_digest_markdown(&digest);
    assert!(
        markdown.starts_with("## ["),
        "a dated heading, so appending accumulates"
    );
    assert!(markdown.contains("| Mean retention |"));
    assert!(
        markdown.ends_with("\n\n"),
        "so the next append does not run on"
    );
}

// --- fixtures ---

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(400)
}

fn fixture() -> (EventLedger, ProjectId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let events: Vec<NormalizedEvent> = (0..3)
        .map(|index| {
            event(
                project,
                worktree,
                index,
                "some captured turn about deployment",
            )
        })
        .collect();
    let evidence: Vec<uuid::Uuid> = events.iter().map(|event| event.event_id).collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "digest-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(3),
        })
        .expect("append");
    for (index, title) in ["a decision", "a fact"].into_iter().enumerate() {
        brain_cli::remember(
            &mut ledger,
            brain_cli::RememberRequest {
                project_id: project,
                worktree_id: worktree,
                kind: MemoryKind::Decision,
                title,
                content: "body",
                evidence_ids: vec![evidence[index]],
                supersedes: Vec::new(),
                now: now(),
            },
        )
        .expect("remember");
    }
    (ledger, project)
}

fn event(project: ProjectId, worktree: WorktreeId, offset: i64, content: &str) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    key[1] = 77;
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: format!("session-{offset}"),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: now(),
        observed_at: now(),
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: offset,
        source_schema: "digest:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": content }),
        raw: serde_json::json!({ "content": content }),
    }
}
