//! Finding the claims a later observation should have updated.
//!
//! Karpathy's Ingest operation — *"a single source might touch 10–15 wiki pages"* — is the last one
//! this system did not perform. These tests pin the grouping, because two earlier groupings were
//! tried and both failed in ways worth not repeating:
//!
//! - **By title** (the contradiction grouping) found **zero** candidates across 2,095 subjects: it
//!   needs an identical title, and after the fold almost every subject is a singleton.
//! - **By derived subject page** found **2,477** — noise. Those subjects are single shared *terms*;
//!   `detection` grouped "duplicate source_id detection" with "staging directory age detection".
//!
//! Shared *evidence* is the criterion that works, and it is the one wikilinks already use: two
//! claims citing the same event are demonstrably about the same episode.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_later_claim_resting_on_unseen_evidence_is_a_candidate() {
    let (ledger, project) = fixture();
    let report = brain_cli::propose_revisions(&ledger, project, Some(0.0)).expect("revise");
    assert_eq!(report.candidates.len(), 1);
    let candidate = &report.candidates[0];
    assert_eq!(candidate.older_title, "Hooks work in both harnesses");
    assert_eq!(candidate.newer_title, "Desktop does not fire hooks");
    assert_eq!(
        candidate.unseen_evidence.len(),
        2,
        "the newer claim saw two events the older one never did"
    );
}

#[test]
fn claims_that_share_no_evidence_are_not_paired() {
    // The whole point of the criterion. Two claims can share every word and still be about
    // different episodes; only shared evidence proves they are about the same one.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let events = seed(&mut ledger, project, worktree, 6);
    append(
        &mut ledger,
        project,
        worktree,
        "Hooks",
        "one",
        &events[0..2],
        day(1),
        1,
    );
    append(
        &mut ledger,
        project,
        worktree,
        "Hooks",
        "two",
        &events[2..6],
        day(9),
        2,
    );

    let report = brain_cli::propose_revisions(&ledger, project, Some(0.0)).expect("revise");
    assert!(
        report.candidates.is_empty(),
        "disjoint evidence means these are not the same episode"
    );
}

#[test]
fn a_pair_is_reported_once_however_much_evidence_it_shares() {
    // Two claims sharing three events would otherwise appear three times — once per shared event —
    // which is how a useful list becomes an unreadable one.
    let (ledger, project) = fixture();
    let report = brain_cli::propose_revisions(&ledger, project, Some(0.0)).expect("revise");
    let mut pairs: Vec<(uuid::Uuid, uuid::Uuid)> = report
        .candidates
        .iter()
        .map(|candidate| (candidate.older_id, candidate.newer_id))
        .collect();
    let before = pairs.len();
    pairs.sort();
    pairs.dedup();
    assert_eq!(before, pairs.len(), "each pair appears at most once");
}

#[test]
fn same_day_pairs_are_hidden_by_default() {
    // Measured: 441 of 454 live candidates were written the same day — consolidation windows
    // overlapping, which the fold handles. The 13 spanning a day or more are the ones where
    // something was learnt later.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let events = seed(&mut ledger, project, worktree, 4);
    append(
        &mut ledger,
        project,
        worktree,
        "Deploy",
        "first",
        &events[0..2],
        day(3),
        1,
    );
    append(
        &mut ledger,
        project,
        worktree,
        "Deploy",
        "second",
        &events[0..4],
        day(3),
        2,
    );

    assert!(
        brain_cli::propose_revisions(&ledger, project, None)
            .expect("default")
            .candidates
            .is_empty(),
        "a same-day pair is consolidation overlap, not a revision"
    );
    assert_eq!(
        brain_cli::propose_revisions(&ledger, project, Some(0.0))
            .expect("all")
            .candidates
            .len(),
        1,
        "--all still shows it"
    );
}

#[test]
fn one_new_event_is_not_enough_to_ask_for_a_review() {
    // A candidate list nobody finishes reading is the same as no candidate list.
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let events = seed(&mut ledger, project, worktree, 3);
    append(
        &mut ledger,
        project,
        worktree,
        "Retry",
        "first",
        &events[0..2],
        day(1),
        1,
    );
    append(
        &mut ledger,
        project,
        worktree,
        "Retry",
        "second",
        &events[0..3],
        day(9),
        2,
    );

    let report = brain_cli::propose_revisions(&ledger, project, Some(0.0)).expect("revise");
    assert!(
        report.candidates.is_empty(),
        "one new event is not a revision"
    );
}

// --- fixtures ---

fn day(n: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(400 + n)
}

/// An older claim on two events, and a newer one on those two plus two more.
fn fixture() -> (EventLedger, ProjectId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let events = seed(&mut ledger, project, worktree, 4);
    append(
        &mut ledger,
        project,
        worktree,
        "Hooks work in both harnesses",
        "observed on the CLI",
        &events[0..2],
        day(1),
        1,
    );
    append(
        &mut ledger,
        project,
        worktree,
        "Desktop does not fire hooks",
        "zero deliveries, zero spool entries",
        &events[0..4],
        day(9),
        2,
    );
    (ledger, project)
}

fn seed(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    count: i64,
) -> Vec<uuid::Uuid> {
    let events: Vec<NormalizedEvent> = (0..count)
        .map(|offset| {
            let mut key = [0_u8; 32];
            key[0] = offset as u8;
            key[1] = 23;
            NormalizedEvent {
                event_id: uuid::Uuid::now_v7(),
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: format!("s{offset}"),
                native_turn_id: None,
                event_type: EventType::UserPrompted,
                occurred_at: day(0),
                observed_at: day(0),
                source_locator: "transcript.jsonl".to_owned(),
                source_offset: offset,
                source_schema: "revise:v1".to_owned(),
                raw_hash: key,
                idempotency_key: key,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": "a turn" }),
                raw: serde_json::json!({ "content": "a turn" }),
            }
        })
        .collect();
    let ids = events.iter().map(|event| event.event_id).collect();
    ledger
        .append_batch(&EventBatch {
            source_id: "revise-fixture".to_owned(),
            events,
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(count as u64),
        })
        .expect("append");
    ids
}

#[allow(clippy::too_many_arguments)]
fn append(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    content: &str,
    evidence: &[uuid::Uuid],
    valid_from: time::OffsetDateTime,
    nonce: u8,
) {
    // Distinct first bytes: `projection_file_name` disambiguates same-titled memories with the
    // first eight hex characters, which in a v7 UUID are timestamp bits.
    let mut bytes = *uuid::Uuid::now_v7().as_bytes();
    bytes[0] = nonce;
    bytes[1] = nonce.wrapping_mul(37);
    ledger
        .append_memory(&MemoryRecord {
            id: uuid::Uuid::from_bytes(bytes),
            version_id: uuid::Uuid::now_v7(),
            scope: MemoryScope::Project(project),
            worktree_id: Some(worktree),
            task_id: None,
            kind: MemoryKind::Fact,
            title: title.to_owned(),
            content: content.to_owned(),
            valid_from,
            valid_to: None,
            recorded_at: valid_from,
            confidence: 1.0,
            authority: Authority::DerivedMemory,
            evidence_ids: evidence.to_vec(),
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        })
        .expect("append memory");
}

// ---------------------------------------------------------------------------
// The rewrite half, exercised against a stub
// ---------------------------------------------------------------------------
//
// The live generation is deliberately held until provider quota returns — watching the citation
// check refuse a *real* bad citation from a *real* provider is the point of having the check, and
// shipping it against a stub while calling it verified would invert that argument. Everything
// between the proposal and the ledger is exercised here.

struct Stub(String);

#[async_trait::async_trait]
impl brain_context::MergeProvider for Stub {
    async fn merge(&self, _instruction: &str) -> anyhow::Result<String> {
        Ok(self.0.clone())
    }
}

fn candidates(ledger: &EventLedger, project: ProjectId) -> Vec<brain_cli::RevisionCandidate> {
    brain_cli::propose_revisions(ledger, project, Some(0.0))
        .expect("revise")
        .candidates
}

#[tokio::test]
async fn a_sound_merge_replaces_both_claims_and_deletes_nothing() {
    let (mut ledger, project) = fixture();
    let found = candidates(&ledger, project);
    let evidence: Vec<String> = ledger
        .current_memory(found[0].newer_id)
        .expect("newer")
        .expect("present")
        .evidence_ids
        .iter()
        .map(|id| id.to_string())
        .collect();
    let body = format!(
        r#"{{"title":"Hooks fire on the CLI only","content":"Short.","evidence_ids":{}}}"#,
        serde_json::to_string(&evidence).expect("json")
    );

    let records_before = ledger.memory_count().expect("before");
    let report =
        brain_cli::merge_candidates(&mut ledger, project, &Stub(body), &found, 10, true, day(20))
            .await
            .expect("merge");

    assert_eq!(report.applied, 1);
    assert_eq!(report.refused, 0);
    let current = ledger.current_project_memories().expect("after");
    assert_eq!(current.len(), 1, "the pair becomes one claim");
    assert_eq!(current[0].title, "Hooks fire on the CLI only");
    assert_eq!(
        ledger.memory_count().expect("after"),
        records_before + 1,
        "a merge appends a record; it never removes one"
    );
}

#[tokio::test]
async fn propose_without_apply_writes_nothing() {
    // The whole point of a two-step approval: reading the proposals must be free.
    let (mut ledger, project) = fixture();
    let found = candidates(&ledger, project);
    let evidence: Vec<String> = ledger
        .current_memory(found[0].newer_id)
        .expect("newer")
        .expect("present")
        .evidence_ids
        .iter()
        .map(|id| id.to_string())
        .collect();
    let body = format!(
        r#"{{"title":"Merged","content":"Short.","evidence_ids":{}}}"#,
        serde_json::to_string(&evidence).expect("json")
    );

    let report = brain_cli::merge_candidates(
        &mut ledger,
        project,
        &Stub(body),
        &found,
        10,
        false,
        day(20),
    )
    .await
    .expect("merge");

    assert_eq!(report.proposed, 1);
    assert_eq!(report.applied, 0);
    assert_eq!(
        ledger.current_project_memories().expect("after").len(),
        2,
        "both claims stay current until --apply"
    );
}

#[tokio::test]
async fn an_unsound_proposal_is_refused_with_its_reason_and_writes_nothing() {
    // A refusal that says nothing is the defect three dead-lettered jobs shipped with: 400
    // characters of plausible JSON and no reason, because `fail()` kept only the top-level message.
    let (mut ledger, project) = fixture();
    let found = candidates(&ledger, project);
    let stranger = uuid::Uuid::now_v7();
    let body = format!(r#"{{"title":"Merged","content":"Short.","evidence_ids":["{stranger}"]}}"#);

    let report =
        brain_cli::merge_candidates(&mut ledger, project, &Stub(body), &found, 10, true, day(20))
            .await
            .expect("merge");

    assert_eq!(report.applied, 0);
    assert_eq!(report.refused, 1);
    let reason = report.outcomes[0].rejected.as_deref().expect("a reason");
    assert!(
        reason.contains("cannot invent provenance"),
        "the refusal must say what was wrong: {reason}"
    );
    assert_eq!(
        ledger.current_project_memories().expect("after").len(),
        2,
        "a refused merge leaves the pair untouched"
    );
}
