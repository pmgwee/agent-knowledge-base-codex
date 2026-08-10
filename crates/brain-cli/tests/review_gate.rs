//! A9's ingest half: a gated memory exists, is cited, and nothing reads it until a person says so.
//!
//! Consolidation batches 200 events to a provider and appends whatever survives validation,
//! unattended. On 10 August that produced merges which passed every mechanical rule — provenance,
//! brevity, evidence retention — and asserted a falsehood, because the claims they rested on
//! asserted it. The rules are about form; this is about truth, so the check has to be a person.
//!
//! The gate is a status rather than a queue table, which is why these tests assert *invisibility*
//! rather than the presence of a row. `MemoryStatus::Proposed` fails `CURRENT_CLAIM`, so the
//! orientation, `search` and the projection all skip it for free — but a test that only checked
//! "the memory was written as proposed" would pass even if every read path ignored the status.

use brain_domain::{
    EventBatch, EventType, Harness, MemoryKind, MemoryStatus, NormalizedEvent, ProjectId,
    SourceCursor, WorktreeId,
};
use brain_service::{
    ConsolidationCrashPoint, ConsolidationLlm, ConsolidationWorker, EvidencePacket, ProposedMemory,
    ProposedMemoryBatch, ReviewGateConfig, WorkerOutcome,
};
use brain_store::{ConsolidationReason, EventLedger};

/// Proposes one decision and one timeline memory, so the gate has something to discriminate on.
struct TwoKinds;

#[async_trait::async_trait]
impl ConsolidationLlm for TwoKinds {
    async fn propose(&self, packet: &EvidencePacket) -> anyhow::Result<ProposedMemoryBatch> {
        let evidence: Vec<uuid::Uuid> = packet.events.iter().map(|e| e.event_id).collect();
        Ok(ProposedMemoryBatch {
            memories: vec![
                ProposedMemory {
                    kind: MemoryKind::Decision,
                    title: "We decided to keep the static CRT".to_owned(),
                    content: "A decision worth a second opinion.".to_owned(),
                    valid_from: time::OffsetDateTime::UNIX_EPOCH,
                    confidence: 1.0,
                    evidence_ids: evidence.clone(),
                    supersedes: Vec::new(),
                },
                ProposedMemory {
                    kind: MemoryKind::Timeline,
                    title: "Events happened in this order".to_owned(),
                    content: "A timeline nobody needs to approve.".to_owned(),
                    valid_from: time::OffsetDateTime::UNIX_EPOCH,
                    confidence: 1.0,
                    evidence_ids: evidence,
                    supersedes: Vec::new(),
                },
            ],
        })
    }
}

fn ledger_with_one_job() -> (EventLedger, time::OffsetDateTime) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: WorktreeId(uuid::Uuid::now_v7()),
                task_id: None,
                harness: Harness::Codex,
                native_session_id: "session".to_owned(),
                native_turn_id: None,
                event_type: EventType::CheckpointAuthored,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture".to_owned(),
                source_offset: 1,
                source_schema: "fixture".to_owned(),
                raw_hash: [7; 32],
                idempotency_key: [7; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": "static CRT"}),
                raw: serde_json::json!({"content": "static CRT"}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
    let job = ledger
        .enqueue_consolidation_job(event_id, event_id, ConsolidationReason::ExplicitCheckpoint)
        .expect("enqueue");
    // The job's own `available_at`, not the epoch: a lease taken before a job is available
    // returns `Idle`, and the assertion would then blame the gate for a scheduling detail.
    (ledger, job.available_at)
}

fn gate(kinds: &[&str]) -> ReviewGateConfig {
    ReviewGateConfig {
        gated_kinds: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
    }
}

async fn consolidate(
    ledger: &mut EventLedger,
    review: ReviewGateConfig,
    now: time::OffsetDateTime,
) {
    let worker =
        ConsolidationWorker::new("test", time::Duration::seconds(30)).with_review_gate(review);
    let outcome = worker
        .run_once(ledger, &TwoKinds, now, ConsolidationCrashPoint::None)
        .await
        .expect("consolidate");
    assert!(matches!(outcome, WorkerOutcome::Completed(_)));
}

#[tokio::test]
async fn an_ungated_kind_is_current_immediately() {
    // The default, and the behaviour every existing deployment already has. Gating must be
    // something an operator turns on, never something that appears under them: a review queue
    // nobody drains is a brain that forgets on purpose.
    let (mut ledger, now) = ledger_with_one_job();
    consolidate(&mut ledger, ReviewGateConfig::default(), now).await;

    let current = ledger.current_project_memories().expect("current");
    assert_eq!(
        current.len(),
        2,
        "nothing is held back when nothing is gated"
    );
    assert!(ledger.proposed_memory_ids().expect("pending").is_empty());
}

#[tokio::test]
async fn a_gated_kind_is_written_but_invisible_until_approved() {
    let (mut ledger, now) = ledger_with_one_job();
    consolidate(&mut ledger, gate(&["decision"]), now).await;

    // Written — the evidence is not thrown away while it waits.
    let pending = ledger.proposed_memory_ids().expect("pending");
    assert_eq!(pending.len(), 1);

    // And invisible. This is the assertion that matters: `current_project_memories` feeds the
    // session-start orientation, the Markdown projection and `brain export`, so a gate the status
    // filter did not enforce would be a gate in name only.
    let current = ledger.current_project_memories().expect("current");
    assert_eq!(current.len(), 1, "only the ungated timeline memory is live");
    assert_eq!(current[0].kind, MemoryKind::Timeline);

    let held = ledger
        .current_memory(pending[0])
        .expect("held")
        .expect("present");
    assert_eq!(held.status, MemoryStatus::Proposed);
    assert_eq!(held.title, "We decided to keep the static CRT");
    assert_eq!(
        held.evidence_ids.len(),
        1,
        "it keeps its provenance while it waits"
    );
}

#[tokio::test]
async fn approving_makes_it_current_by_appending_a_version() {
    let (mut ledger, now) = ledger_with_one_job();
    consolidate(&mut ledger, gate(&["decision"]), now).await;
    let id = ledger.proposed_memory_ids().expect("pending")[0];
    let memories_before = ledger.memory_count().expect("before");
    let versions_before = ledger.memory_versions(id).expect("versions").len();

    brain_cli::rule_on_memory(&mut ledger, id, true, now).expect("approve");

    // A ruling appends a *version* of the same memory — the identity does not change, so the
    // memory count must not move. Asserting on the memory count alone would pass even if the
    // proposal had been rewritten in place, which is the one thing an append-only ledger forbids.
    assert_eq!(ledger.memory_count().expect("after"), memories_before);
    assert_eq!(
        ledger.memory_versions(id).expect("versions").len(),
        versions_before + 1,
        "the ruling is appended; the proposal it rules on is still there"
    );
    assert!(ledger.proposed_memory_ids().expect("pending").is_empty());
    let current = ledger.current_project_memories().expect("current");
    assert_eq!(current.len(), 2, "the approved decision joins the timeline");
}

#[tokio::test]
async fn rejecting_keeps_the_claim_and_the_fact_that_someone_said_no() {
    // Not a delete. The ledger records that a person looked and declined, which is exactly what
    // the next consolidation proposing the same claim needs to know.
    let (mut ledger, now) = ledger_with_one_job();
    consolidate(&mut ledger, gate(&["decision"]), now).await;
    let id = ledger.proposed_memory_ids().expect("pending")[0];

    brain_cli::rule_on_memory(&mut ledger, id, false, now).expect("reject");

    assert!(ledger.proposed_memory_ids().expect("pending").is_empty());
    assert_eq!(
        ledger.current_project_memories().expect("current").len(),
        1,
        "a rejected claim never becomes current"
    );
    let kept = ledger.current_memory(id).expect("kept").expect("present");
    assert_eq!(kept.status, MemoryStatus::Invalid);
    assert_eq!(kept.title, "We decided to keep the static CRT");
}

#[tokio::test]
async fn only_a_proposal_can_be_ruled_on() {
    // Approving an already-current memory would append a redundant version; "approving" a
    // superseded one would quietly revive a retired claim. Same refusal, same reason, as the
    // reviewed-merge path in `revise`.
    let (mut ledger, now) = ledger_with_one_job();
    consolidate(&mut ledger, gate(&["decision"]), now).await;
    let id = ledger.proposed_memory_ids().expect("pending")[0];
    brain_cli::rule_on_memory(&mut ledger, id, true, now).expect("approve");

    let again = brain_cli::rule_on_memory(&mut ledger, id, true, now);
    let error = again.expect_err("a second ruling is refused").to_string();
    assert!(
        error.contains("not proposed"),
        "the refusal should say why: {error}"
    );
}
