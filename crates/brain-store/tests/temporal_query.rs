use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery, SearchSource};

#[test]
fn last_week_uses_occurrence_time_and_reports_late_observation() {
    let now = time::OffsetDateTime::from_unix_timestamp(1_775_299_200).expect("fixed now");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let in_range = uuid::Uuid::now_v7();
    let outside = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![
                event(
                    in_range,
                    project,
                    worktree,
                    now - time::Duration::days(5),
                    now,
                    1,
                    "worked last week ingested today",
                ),
                event(
                    outside,
                    project,
                    worktree,
                    now - time::Duration::days(8),
                    now - time::Duration::days(8),
                    2,
                    "outside rolling week",
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append events");

    let hits = ledger
        .search(&SearchQuery::last_week(project, now))
        .expect("search last week");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source_id, in_range);
    assert!(hits[0].late_observation);
}

#[test]
fn as_of_queries_return_the_version_that_was_valid_then() {
    let now = time::OffsetDateTime::from_unix_timestamp(1_775_299_200).expect("fixed now");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let old_evidence = uuid::Uuid::now_v7();
    let new_evidence = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![
                event(
                    old_evidence,
                    project,
                    worktree,
                    now - time::Duration::days(10),
                    now - time::Duration::days(10),
                    1,
                    "Redis decision evidence",
                ),
                event(
                    new_evidence,
                    project,
                    worktree,
                    now - time::Duration::days(2),
                    now - time::Duration::days(2),
                    2,
                    "SQLite decision evidence",
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append evidence");
    let memory_id = uuid::Uuid::now_v7();
    let old = memory(
        memory_id,
        project,
        worktree,
        now - time::Duration::days(10),
        "Use Redis for cache",
        old_evidence,
        Vec::new(),
    );
    ledger.append_memory(&old).expect("append old memory");
    let new = memory(
        memory_id,
        project,
        worktree,
        now - time::Duration::days(2),
        "Use SQLite for cache",
        new_evidence,
        vec![old.version_id],
    );
    ledger.append_memory(&new).expect("append new memory");

    let then = ledger
        .search(
            &SearchQuery::text(project, "cache")
                .as_of(now - time::Duration::days(8))
                .memories_only(),
        )
        .expect("search historical memory");
    assert_eq!(then.len(), 1);
    assert_eq!(then[0].source, SearchSource::Memory);
    assert!(then[0].text.contains("Redis"));

    let current = ledger
        .search(
            &SearchQuery::text(project, "cache")
                .as_of(now)
                .memories_only(),
        )
        .expect("search current memory");
    assert_eq!(current.len(), 1);
    assert!(current[0].text.contains("SQLite"));
}

fn event(
    event_id: uuid::Uuid,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    occurred_at: time::OffsetDateTime,
    observed_at: time::OffsetDateTime,
    sequence: u8,
    content: &str,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id,
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::Codex,
        native_session_id: "temporal-session".to_owned(),
        native_turn_id: None,
        event_type: EventType::AgentResponded,
        occurred_at,
        observed_at,
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: i64::from(sequence),
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [sequence; 32],
        idempotency_key: [sequence.saturating_add(20); 32],
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({"content": content}),
        raw: serde_json::json!({"content": content}),
    }
}

fn memory(
    id: uuid::Uuid,
    project: ProjectId,
    worktree: WorktreeId,
    valid_from: time::OffsetDateTime,
    content: &str,
    evidence_id: uuid::Uuid,
    supersedes: Vec<uuid::Uuid>,
) -> MemoryRecord {
    MemoryRecord {
        id,
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: "Cache backend".to_owned(),
        content: content.to_owned(),
        valid_from,
        valid_to: None,
        recorded_at: valid_from,
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: vec![evidence_id],
        supersedes,
        status: MemoryStatus::Current,
    }
}
