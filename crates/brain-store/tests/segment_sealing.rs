use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery, SegmentCatalog, SegmentStore, should_seal};
use sha2::{Digest, Sha256};

#[test]
fn sealed_segment_is_deterministic_verified_and_catalogued_without_losing_hot_rows() {
    let fixture = fixture();
    let store = SegmentStore::new(
        &fixture.ledger_path,
        fixture.temp.path().join("segments"),
        fixture.project,
    )
    .expect("segment store");
    let first = fixture.events[0].event_id;
    let last = fixture.events[2].event_id;
    let before = EventLedger::open(&fixture.ledger_path, fixture.project)
        .expect("ledger")
        .search(&SearchQuery::text(fixture.project, "event 1"))
        .expect("search before");
    let report = store
        .seal(first, last, fixture.now + time::Duration::hours(1))
        .expect("seal");
    let replay = store
        .seal(first, last, fixture.now + time::Duration::hours(2))
        .expect("replay seal");

    assert!(!report.replayed);
    assert!(replay.replayed);
    assert_eq!(report.manifest, replay.manifest);
    assert_eq!(
        store.read(&report.manifest_path).expect("read"),
        fixture.events
    );
    assert_eq!(
        EventLedger::open(&fixture.ledger_path, fixture.project)
            .expect("ledger")
            .event_count()
            .expect("hot count"),
        3
    );
    let catalog = SegmentCatalog::open(&fixture.ledger_path, fixture.project).expect("catalog");
    assert_eq!(
        catalog
            .events(report.manifest.segment_id)
            .expect("events")
            .len(),
        3
    );
    assert_eq!(
        store.segment_open_count(),
        1,
        "only explicit read opened cold data"
    );
    assert_eq!(
        store.compact(report.manifest.segment_id).expect("compact"),
        3
    );
    let after = EventLedger::open(&fixture.ledger_path, fixture.project)
        .expect("ledger")
        .search(&SearchQuery::text(fixture.project, "event 1"))
        .expect("search after");
    assert_eq!(
        before.iter().map(|hit| hit.source_id).collect::<Vec<_>>(),
        after.iter().map(|hit| hit.source_id).collect::<Vec<_>>()
    );
    assert_eq!(
        store
            .event(fixture.events[1].event_id)
            .expect("tiered event"),
        Some(fixture.events[1].clone())
    );
}

#[test]
fn corrupted_segment_is_detected_without_removing_canonical_events() {
    let fixture = fixture();
    let store = SegmentStore::new(
        &fixture.ledger_path,
        fixture.temp.path().join("segments"),
        fixture.project,
    )
    .expect("segment store");
    let report = store
        .seal(
            fixture.events[0].event_id,
            fixture.events[2].event_id,
            fixture.now,
        )
        .expect("seal");
    std::fs::write(&report.data_path, b"corrupt").expect("corrupt segment");
    assert!(store.read(&report.manifest_path).is_err());
    assert_eq!(
        EventLedger::open(&fixture.ledger_path, fixture.project)
            .expect("ledger")
            .event_count()
            .expect("count"),
        3
    );
}

#[test]
fn sealing_threshold_covers_size_and_month_boundaries() {
    let january = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let next_month = january + time::Duration::days(32);
    assert!(should_seal(256 * 1024 * 1024, january, january));
    assert!(should_seal(1, january, next_month));
    assert!(!should_seal(1, january, january));
}

struct Fixture {
    temp: tempfile::TempDir,
    ledger_path: std::path::PathBuf,
    project: ProjectId,
    now: time::OffsetDateTime,
    events: Vec<brain_store::StoredEvent>,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temp");
    let ledger_path = temp.path().join("ledger.sqlite");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let normalized = (0..3)
        .map(|index| {
            event(
                project,
                worktree,
                now + time::Duration::minutes(index),
                index,
            )
        })
        .collect::<Vec<_>>();
    let mut ledger = EventLedger::open(&ledger_path, project).expect("ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: normalized.clone(),
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(3),
        })
        .expect("append");
    let events = ledger
        .events_between(normalized[0].event_id, normalized[2].event_id)
        .expect("stored events");
    Fixture {
        temp,
        ledger_path,
        project,
        now,
        events,
    }
}

fn event(
    project: ProjectId,
    worktree: WorktreeId,
    occurred_at: time::OffsetDateTime,
    index: i64,
) -> NormalizedEvent {
    let raw = serde_json::json!({"index": index, "message": format!("event {index}")});
    let raw_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&raw).expect("raw")).into();
    let idempotency_key: [u8; 32] = Sha256::digest(format!("fixture:{index}")).into();
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::Codex,
        native_session_id: "scale-fixture".to_owned(),
        native_turn_id: Some(index.to_string()),
        event_type: EventType::AgentResponded,
        occurred_at,
        observed_at: occurred_at,
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: index,
        source_schema: "fixture:v1".to_owned(),
        raw_hash,
        idempotency_key,
        git_head: Some("abc123".to_owned()),
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({"message": format!("event {index}")}),
        raw,
    }
}
