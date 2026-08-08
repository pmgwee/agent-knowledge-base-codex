//! Folding a contradiction, by supersession.
//!
//! The operation Karpathy's pattern names and this vault had never performed: *"doesn't just index
//! it for later retrieval — it integrates it into the existing wiki."* Consolidation re-derives the
//! same claim from overlapping event windows and files each one as a new memory, so the vault
//! accumulated duplicates rather than revisions. Measured on the live corpus before this shipped:
//! 2,097 memories, 2,097 distinct ids, **zero supersession edges** — the whole lifecycle was schema
//! and reader filters that production had never once exercised.
//!
//! These tests pin the two properties that make folding safe to run unattended: it only ever
//! appends, and it refuses the cases derivation cannot separate.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn folding_retires_the_duplicates_and_keeps_one_current() {
    let (mut ledger, project) = fixture();
    assert_eq!(ledger.current_project_memories().expect("before").len(), 3);

    let applied = brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("apply");
    assert_eq!(applied.folded, 1);
    assert_eq!(applied.superseded, 2);
    assert_eq!(applied.left_for_you, 0);

    let current = ledger.current_project_memories().expect("after");
    assert_eq!(current.len(), 1, "three duplicates fold to one claim");
    assert_eq!(current[0].title, "The deploy runs on commit");
}

#[test]
fn nothing_is_deleted_by_a_fold() {
    // The invariant the whole ledger rests on. A fold is two appends: a new version of the keeper
    // carrying the edges, and a retirement version per loser. Rows only ever grow.
    let (mut ledger, project) = fixture();
    let versions_before = ledger.memory_version_count().expect("before");

    brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("apply");

    let versions_after = ledger.memory_version_count().expect("after");
    assert!(
        versions_after > versions_before,
        "a fold appends; it never removes ({versions_before} -> {versions_after})"
    );
    assert_eq!(
        ledger.memory_count().expect("records"),
        3,
        "the memory records themselves are untouched — only their current version changed"
    );
}

#[test]
fn the_keeper_records_what_it_absorbed() {
    // Without this the fold is indistinguishable from a deletion: the losers vanish from every
    // read path and nothing says where they went.
    let (mut ledger, project) = fixture();
    brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("apply");

    let counts = ledger.revision_counts().expect("counts");
    assert_eq!(counts.len(), 1, "one memory absorbed the others");
    assert_eq!(counts.values().sum::<u64>(), 2);
}

#[test]
fn every_counting_read_path_agrees_after_a_fold() {
    // The defect the first live fold exposed, pinned so it cannot come back. Eleven queries
    // selected versions by `status = 'current'`, which was indistinguishable from "the memory's
    // latest version" while every memory had exactly one. A fold breaks that in both directions at
    // once: the keeper gains a second current version and is counted twice, and the loser keeps its
    // original current version and never leaves. Live, that read as 2,101 memories in `brain
    // digest` against 2,090 in `brain lint`.
    let (mut ledger, project) = fixture();
    brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("apply");

    let current = ledger.current_project_memories().expect("current").len();
    let retention = ledger.memory_retention(later()).expect("retention").len();
    assert_eq!(
        current, retention,
        "the projection and the retention curve must count the same corpus"
    );
    assert_eq!(
        current, 1,
        "three duplicates fold to one claim, counted once"
    );
}

#[test]
fn folding_twice_changes_nothing_the_second_time() {
    // A scheduled run must be safe to repeat. After the first fold there is no contradiction left,
    // so the second pass has nothing to find.
    let (mut ledger, project) = fixture();
    brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("first");
    let second = brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("second");
    assert_eq!(second.folded, 0);
    assert_eq!(second.superseded, 0);
}

#[test]
fn a_level_contradiction_is_left_alone() {
    // Same authority, same day, same evidence count. Derivation cannot separate these, and a
    // coin-flip dressed as a rule would be applied silently and never revisited.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let evidence = seed_events(&mut ledger, project, worktree, 2);
    for (index, content) in ["retries are three", "retries are five"]
        .into_iter()
        .enumerate()
    {
        append(
            &mut ledger,
            project,
            worktree,
            "Retry policy",
            content,
            vec![evidence[index]],
            base(),
            index as u8 + 40,
        );
    }

    let applied = brain_cli::apply_reconciliation(&mut ledger, project, later()).expect("apply");
    assert_eq!(applied.folded, 0);
    assert_eq!(applied.left_for_you, 1);
    assert_eq!(
        ledger.current_project_memories().expect("after").len(),
        2,
        "both sides stay current until a person decides"
    );
}

// --- fixtures ---

fn base() -> time::OffsetDateTime {
    time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(400)
}

fn later() -> time::OffsetDateTime {
    base() + time::Duration::days(10)
}

/// Three memories with the same title and differing bodies, separable by date — the exact shape
/// the live corpus had.
///
/// The bodies must differ. `resolve_candidates` reports a conflict only when normalised *content*
/// differs within a subject, which is correct: identical text under one title is a duplicate, not a
/// disagreement. The live contradictions were re-derivations of one claim from overlapping event
/// windows, so each phrased it slightly differently — same subject, same conclusion, different
/// words. That is what this reproduces.
fn fixture() -> (EventLedger, ProjectId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let evidence = seed_events(&mut ledger, project, worktree, 3);
    let bodies = [
        "the post-commit hook builds and installs",
        "committing triggers a build, then an install",
        "a commit runs the build and then installs the binaries",
    ];
    for (index, offset) in [0_i64, 1, 2].into_iter().enumerate() {
        append(
            &mut ledger,
            project,
            worktree,
            "The deploy runs on commit",
            bodies[index],
            vec![evidence[index]],
            base() + time::Duration::days(offset),
            index as u8 + 1,
        );
    }
    (ledger, project)
}

fn seed_events(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    count: i64,
) -> Vec<uuid::Uuid> {
    let events: Vec<NormalizedEvent> = (0..count)
        .map(|offset| {
            let mut key = [0_u8; 32];
            key[0] = offset as u8;
            key[1] = 61;
            NormalizedEvent {
                event_id: uuid::Uuid::now_v7(),
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: format!("session-{offset}"),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: base(),
                observed_at: base(),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "fold:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": "a turn" }),
                raw: serde_json::json!({ "content": "a turn" }),
            }
        })
        .collect();
    let ids: Vec<uuid::Uuid> = events.iter().map(|event| event.event_id).collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "fold-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(count as u64),
        })
        .expect("append");
    ids
}

/// A memory id whose first eight hex characters are distinct.
///
/// `projection_file_name` disambiguates same-titled memories with exactly those characters, and in
/// a v7 UUID they are *timestamp* bits — so three same-titled memories minted in one millisecond
/// collide on the `projection_path` unique index. That is the narrow collision the status doc
/// records as never observed; it is observable here because these fixtures mint in a tight loop.
/// It fails loudly rather than overwriting, which is the correct behaviour and why this only ever
/// bit a test.
fn distinct_id(nonce: u8) -> uuid::Uuid {
    let mut bytes = *uuid::Uuid::now_v7().as_bytes();
    bytes[0] = nonce;
    bytes[1] = nonce.wrapping_mul(31);
    uuid::Uuid::from_bytes(bytes)
}

#[allow(clippy::too_many_arguments)]
fn append(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    content: &str,
    evidence_ids: Vec<uuid::Uuid>,
    valid_from: time::OffsetDateTime,
    nonce: u8,
) {
    ledger
        .append_memory(&MemoryRecord {
            id: distinct_id(nonce),
            version_id: uuid::Uuid::now_v7(),
            scope: MemoryScope::Project(project),
            worktree_id: Some(worktree),
            task_id: None,
            kind: MemoryKind::Decision,
            title: title.to_owned(),
            content: content.to_owned(),
            valid_from,
            valid_to: None,
            recorded_at: valid_from,
            confidence: 1.0,
            // Derived, so authority ties and the rules fall through to recency — which is what the
            // live contradictions did.
            authority: Authority::DerivedMemory,
            evidence_ids,
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        })
        .expect("append memory");
}
