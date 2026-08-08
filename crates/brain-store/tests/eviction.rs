//! Eviction, and the refusals that make it safe.
//!
//! The gate is the feature. Everything else here is a consequence of it.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, EvictionGate, SearchQuery};

const DAY: time::Duration = time::Duration::days(1);

#[test]
fn eviction_refuses_until_access_has_been_counted_long_enough() {
    // The defect this exists to prevent: on the day counting ships every memory is
    // never-retrieved, so a policy reading that number retires the entire corpus and produces a
    // defensible reason for each deletion.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    add_memory(
        &mut ledger,
        project,
        worktree,
        "Old claim",
        epoch - 200 * DAY,
    );

    let plan = ledger.plan_eviction(epoch + DAY).expect("plan");
    assert!(matches!(plan.gate, EvictionGate::TooYoung { .. }));
    assert!(
        !plan.candidates.is_empty(),
        "a closed gate must still show what it would take — a policy nobody can preview is a \
         policy nobody can review"
    );

    let error = ledger
        .apply_eviction(epoch + DAY, "test")
        .expect_err("applying before the gate opens must fail");
    let message = format!("{error}");
    assert!(
        message.contains("29") || message.contains("30"),
        "the refusal should say how long is left, got: {message}"
    );

    // And nothing was touched.
    assert_eq!(ledger.tombstones().expect("tombstones").len(), 0);
}

#[test]
fn once_the_gate_opens_a_never_retrieved_memory_is_retired_as_a_tombstone() {
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    let id = add_memory(
        &mut ledger,
        project,
        worktree,
        "Unused claim",
        epoch - 200 * DAY,
    );

    let now = epoch + 31 * DAY;
    let plan = ledger.plan_eviction(now).expect("plan");
    assert!(plan.gate.is_open());
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(plan.candidates[0].memory_id, id);

    let retired = ledger.apply_eviction(now, "test").expect("apply");
    assert_eq!(retired, 1);

    // Tombstoned, not deleted: the row is still there and the withdrawal is on the record.
    let tombstone = ledger
        .tombstone(id)
        .expect("tombstone lookup")
        .expect("the memory must be tombstoned");
    assert!(tombstone.reason.contains("never retrieved"));
    assert!(
        ledger
            .search(&SearchQuery::text(project, "Unused").memories_only())
            .expect("search")
            .is_empty(),
        "an evicted memory must stop being returned"
    );
}

#[test]
fn a_memory_retrieval_has_reached_is_never_a_candidate() {
    // The whole premise: use is the signal. One retrieval is enough to take a memory off the list,
    // and it must keep it off without anything having to clear a flag.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    let id = add_memory(
        &mut ledger,
        project,
        worktree,
        "Wanted claim",
        epoch - 200 * DAY,
    );

    let now = epoch + 31 * DAY;
    assert_eq!(ledger.plan_eviction(now).expect("plan").candidates.len(), 1);

    ledger
        .record_memory_access(&[id], now - DAY)
        .expect("record access");

    let plan = ledger.plan_eviction(now).expect("plan again");
    assert!(
        plan.candidates.is_empty(),
        "one retrieval must be enough to take a memory off the list"
    );
    assert_eq!(ledger.apply_eviction(now, "test").expect("apply"), 0);
}

#[test]
fn a_claim_a_human_filed_is_protected_from_disuse() {
    // `brain remember` promises that filing a conclusion keeps it. Retiring one for going unread
    // would quietly make that promise false, and the user would have no way to notice.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    let id = add_memory_with_authority(
        &mut ledger,
        project,
        worktree,
        "Human filed this",
        epoch - 200 * DAY,
        Authority::HumanCorrection,
    );

    let now = epoch + 31 * DAY;
    let plan = ledger.plan_eviction(now).expect("plan");
    assert!(plan.candidates.is_empty());
    assert_eq!(plan.protected, 1);
    assert_eq!(ledger.apply_eviction(now, "test").expect("apply"), 0);
    assert!(ledger.tombstone(id).expect("lookup").is_none());
}

#[test]
fn a_memory_younger_than_the_quiet_period_is_left_alone() {
    // Disuse of something recorded last week says nothing about it.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    add_memory(
        &mut ledger,
        project,
        worktree,
        "Recent claim",
        epoch + 25 * DAY,
    );

    let plan = ledger.plan_eviction(epoch + 31 * DAY).expect("plan");
    assert!(plan.gate.is_open());
    assert!(
        plan.candidates.is_empty(),
        "90 days of quiet is required, and this memory is six days old"
    );
}

#[test]
fn applying_re_plans_so_a_memory_used_since_the_preview_survives() {
    // A plan is a snapshot. Between showing it and agreeing to it, retrieval may reach one of these
    // — which is exactly the signal it should not be retired. Acting on the stale list would delete
    // the thing that just proved itself useful.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    let keep = add_memory(
        &mut ledger,
        project,
        worktree,
        "Claim one",
        epoch - 200 * DAY,
    );
    let drop = add_memory(
        &mut ledger,
        project,
        worktree,
        "Claim two",
        epoch - 200 * DAY,
    );

    let now = epoch + 31 * DAY;
    assert_eq!(ledger.plan_eviction(now).expect("plan").candidates.len(), 2);

    // Something retrieves one of them after the plan was shown.
    ledger.record_memory_access(&[keep], now).expect("access");

    assert_eq!(ledger.apply_eviction(now, "test").expect("apply"), 1);
    assert!(ledger.tombstone(keep).expect("lookup").is_none());
    assert!(ledger.tombstone(drop).expect("lookup").is_some());
}

#[test]
fn the_plan_reports_the_share_it_would_retire() {
    // A policy proposing to retire most of a brain is describing a retrieval problem, and deleting
    // the evidence would remove the only way to see that.
    let (mut ledger, project, worktree) = fixture();
    let epoch = ledger.access_epoch().expect("epoch");
    for index in 0..4 {
        add_memory(
            &mut ledger,
            project,
            worktree,
            &format!("Claim {index}"),
            epoch - 200 * DAY,
        );
    }
    let used = add_memory(
        &mut ledger,
        project,
        worktree,
        "Used claim",
        epoch - 200 * DAY,
    );
    let now = epoch + 31 * DAY;
    ledger.record_memory_access(&[used], now).expect("access");

    let plan = ledger.plan_eviction(now).expect("plan");
    assert_eq!(plan.total_current, 5);
    assert_eq!(plan.candidates.len(), 4);
    assert!((plan.share() - 0.8).abs() < 1e-9);
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "eviction-fixture".to_owned(),
            events: vec![event(project, worktree, 0)],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
    (ledger, project, worktree)
}

fn cited_event(ledger: &EventLedger, project: ProjectId) -> uuid::Uuid {
    ledger
        .search(
            &SearchQuery::text(project, "deployment")
                .events_only()
                .with_limit(1),
        )
        .expect("find an event")
        .first()
        .map(|hit| hit.source_id)
        .expect("the fixture has one event")
}

fn add_memory(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    valid_from: time::OffsetDateTime,
) -> uuid::Uuid {
    add_memory_with_authority(
        ledger,
        project,
        worktree,
        title,
        valid_from,
        Authority::DerivedMemory,
    )
}

fn add_memory_with_authority(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    valid_from: time::OffsetDateTime,
    authority: Authority,
) -> uuid::Uuid {
    let cited = cited_event(ledger, project);
    let record = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("{title} and the reasoning behind it."),
        valid_from,
        valid_to: None,
        recorded_at: valid_from,
        confidence: 1.0,
        authority,
        evidence_ids: vec![cited],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    };
    let id = record.id;
    ledger.append_memory(&record).expect("append memory");
    id
}

fn event(project: ProjectId, worktree: WorktreeId, offset: i64) -> NormalizedEvent {
    let mut key = [0_u8; 32];
    key[0] = offset as u8;
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: format!("session-{offset}"),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: offset,
        source_schema: "eviction:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": "the deployment pipeline ran" }),
        raw: serde_json::json!({ "content": "the deployment pipeline ran" }),
    }
}
