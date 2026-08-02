use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn replaying_one_batch_creates_no_duplicate_events() {
    let batch = fixture_batch();
    let mut ledger =
        EventLedger::open_in_memory(batch.events[0].project_id).expect("open in-memory ledger");

    assert_eq!(
        ledger.append_batch(&batch).expect("append batch").inserted,
        3
    );
    assert_eq!(
        ledger.append_batch(&batch).expect("replay batch").inserted,
        0
    );
    assert_eq!(ledger.event_count().expect("count events"), 3);
    assert_eq!(
        ledger.cursor("claude:fixture").expect("read cursor"),
        SourceCursor::byte_offset(300)
    );
}

#[test]
fn project_time_queries_use_the_scoped_composite_index() {
    let batch = fixture_batch();
    let project_id = batch.events[0].project_id;
    let mut ledger = EventLedger::open_in_memory(project_id).expect("open in-memory ledger");
    ledger.append_batch(&batch).expect("append fixture batch");

    let plan = ledger
        .explain_project_time_query(project_id)
        .expect("inspect query plan");

    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_events_project_occurred")),
        "unexpected SQLite query plan: {plan:?}"
    );
}

fn fixture_batch() -> EventBatch {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let events = (1_u8..=3)
        .map(|sequence| NormalizedEvent {
            event_id: uuid::Uuid::now_v7(),
            project_id,
            worktree_id,
            task_id: None,
            harness: Harness::ClaudeCode,
            native_session_id: "session-1".to_owned(),
            native_turn_id: Some(format!("turn-{sequence}")),
            event_type: EventType::AgentResponded,
            occurred_at: time::OffsetDateTime::UNIX_EPOCH
                + time::Duration::seconds(i64::from(sequence)),
            observed_at: time::OffsetDateTime::UNIX_EPOCH
                + time::Duration::seconds(i64::from(sequence) + 1),
            source_locator: "fixture.jsonl".to_owned(),
            source_offset: i64::from(sequence) * 100,
            source_schema: "claude-jsonl:v1".to_owned(),
            raw_hash: [sequence; 32],
            idempotency_key: [sequence + 10; 32],
            git_head: None,
            git_branch: Some("main".to_owned()),
            payload: serde_json::json!({"sequence": sequence}),
            raw: serde_json::json!({"type": "assistant", "sequence": sequence}),
        })
        .collect();

    EventBatch {
        source_id: "claude:fixture".to_owned(),
        events,
        next_cursor: SourceCursor::byte_offset(300),
    }
}
