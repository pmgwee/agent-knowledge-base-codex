//! Decay as a curve rather than a flag, and use as something that buys survival.
//!
//! The boolean was honest but coarse: it could say a memory *is* stale and never how close another
//! one sits to the line, which is exactly what ranking needs. And it treated one retrieval the same
//! as fifty — counted, never rewarded.
//!
//! Every input here is a fact from the ledger. That is the whole reason decay is allowed to be
//! automatic: nothing in this file is a model's opinion.

use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{
    EventLedger, RETENTION_HALF_LIFE_DAYS, STALE_RETENTION, SearchQuery, retention_score,
};

const DAY: time::Duration = time::Duration::days(1);

#[test]
fn a_fresh_memory_is_fully_retained_and_decays_from_there() {
    let (fresh, _) = retention_score(0, 0.0);
    assert!((fresh - 1.0).abs() < 1e-9, "no elapsed time means no decay");

    let (half, _) = retention_score(0, RETENTION_HALF_LIFE_DAYS);
    assert!(
        (half - STALE_RETENTION).abs() < 1e-6,
        "one half-life unused should land exactly on the stale line, got {half}"
    );

    let (older, _) = retention_score(0, RETENTION_HALF_LIFE_DAYS * 3.0);
    assert!(older < half, "retention must fall monotonically with quiet");
}

#[test]
fn use_buys_survival_rather_than_merely_resetting_a_clock() {
    // The half this adds. Two memories quiet for exactly as long, one of which has been wanted ten
    // times: the used one must still be retained where the unused one has gone stale.
    let quiet = RETENTION_HALF_LIFE_DAYS * 1.5;
    let (never, s0) = retention_score(0, quiet);
    let (often, s10) = retention_score(10, quiet);

    assert!(never < STALE_RETENTION, "unused and old is stale");
    assert!(
        often > STALE_RETENTION,
        "the same age, but wanted ten times, should survive: {often}"
    );
    assert!(
        s10 > s0 * 3.0,
        "ten retrievals should be worth ~3.4x, got {s10} against {s0}"
    );
}

#[test]
fn the_tenth_retrieval_matters_less_than_the_first() {
    // Logarithmic on purpose. Linear strengthening would make one hot memory permanent and let it
    // crowd out everything the corpus learned afterwards.
    let (_, s1) = retention_score(1, 0.0);
    let (_, s2) = retention_score(2, 0.0);
    let (_, s50) = retention_score(50, 0.0);
    let (_, s51) = retention_score(51, 0.0);

    let first_step = s2 - s1;
    let fiftieth_step = s51 - s50;
    assert!(
        first_step > fiftieth_step * 5.0,
        "diminishing returns: first step {first_step}, fiftieth {fiftieth_step}"
    );
}

#[test]
fn retention_is_reported_per_memory_from_the_ledger() {
    let (mut ledger, project, worktree) = fixture();
    let old = memory(&mut ledger, project, worktree, "Old and unwanted", 300);
    let used = memory(&mut ledger, project, worktree, "Old but wanted", 300);

    let now = time::OffsetDateTime::UNIX_EPOCH + DAY * 300;
    for _ in 0..12 {
        ledger
            .record_memory_access(&[used], now - DAY)
            .expect("record access");
    }

    let scored = ledger.memory_retention(now).expect("retention");
    let find = |id| {
        scored
            .iter()
            .find(|r| r.memory_id == id)
            .copied()
            .expect("scored")
    };
    let old = find(old);
    let used = find(used);

    assert!(old.is_stale(), "never retrieved and 300 days old");
    assert!(
        !used.is_stale(),
        "retrieved twelve times yesterday: retention {}",
        used.retention
    );
    assert!(used.retention > old.retention);
    assert_eq!(used.retrieved_count, 12);
    assert!(
        used.quiet_days < 2.0,
        "quiet is measured from last use, not from creation: {}",
        used.quiet_days
    );
}

#[test]
fn the_curve_and_the_flag_agree_on_the_same_corpus() {
    // Two mechanisms answering one question must not disagree, or a note marked stale in the vault
    // could rank as fresh in the orientation — worse than either being wrong alone.
    let (mut ledger, project, worktree) = fixture();
    memory(&mut ledger, project, worktree, "Ancient", 400);
    memory(&mut ledger, project, worktree, "Recent", 5);

    let now = time::OffsetDateTime::UNIX_EPOCH + DAY * 400;
    let flagged = ledger
        .stale_memory_ids(now, time::Duration::days(90))
        .expect("flags");
    let scored = ledger.memory_retention(now).expect("retention");

    for entry in &scored {
        assert_eq!(
            entry.is_stale(),
            flagged.contains(&entry.memory_id),
            "curve and flag disagree for {}: retention {}",
            entry.memory_id,
            entry.retention
        );
    }
}

// --- fixtures ---

fn fixture() -> (EventLedger, ProjectId, WorktreeId) {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open");
    ledger
        .append_batch(&EventBatch {
            source_id: "retention-fixture".to_owned(),
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
    age_days: i64,
) -> uuid::Uuid {
    let cited = ledger
        .search(
            &SearchQuery::text(project, "deployment")
                .events_only()
                .with_limit(1),
        )
        .expect("search")
        .first()
        .map(|hit| hit.source_id)
        .expect("one event");
    let valid_from = time::OffsetDateTime::UNIX_EPOCH + DAY * ((400 - age_days) as i32);
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
    let key = [23_u8; 32];
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "retention".to_owned(),
        native_turn_id: None,
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "transcript.jsonl".to_owned(),
        source_offset: 0,
        source_schema: "retention:v1".to_owned(),
        raw_hash: key,
        idempotency_key: key,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({ "content": "the deployment pipeline ran" }),
        raw: serde_json::json!({ "content": "the deployment pipeline ran" }),
    }
}
