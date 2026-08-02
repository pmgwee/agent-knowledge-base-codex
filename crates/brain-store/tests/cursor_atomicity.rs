use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn cursor_does_not_advance_when_event_insert_fails() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).expect("open in-memory ledger");
    let original = event(project_id, uuid::Uuid::now_v7(), [1; 32], 100);
    ledger
        .append_batch(&EventBatch {
            source_id: "claude:fixture".to_owned(),
            events: vec![original.clone()],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(100),
        })
        .expect("append original event");

    let conflicting = event(project_id, original.event_id, [2; 32], 200);
    let result = ledger.append_batch(&EventBatch {
        source_id: "claude:fixture".to_owned(),
        events: vec![conflicting],
        quarantined: Vec::new(),
        capture_gaps: Vec::new(),
        next_cursor: SourceCursor::byte_offset(200),
    });

    assert!(result.is_err());
    assert_eq!(
        ledger.cursor("claude:fixture").expect("read cursor"),
        SourceCursor::byte_offset(100)
    );
    assert_eq!(ledger.event_count().expect("count events"), 1);
}

fn event(
    project_id: ProjectId,
    event_id: uuid::Uuid,
    idempotency_key: [u8; 32],
    source_offset: i64,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id,
        project_id,
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "session-1".to_owned(),
        native_turn_id: Some("turn-1".to_owned()),
        event_type: EventType::AgentResponded,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "fixture.jsonl".to_owned(),
        source_offset,
        source_schema: "claude-jsonl:v1".to_owned(),
        raw_hash: [3; 32],
        idempotency_key,
        git_head: None,
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({}),
        raw: serde_json::json!({"type": "assistant"}),
    }
}
