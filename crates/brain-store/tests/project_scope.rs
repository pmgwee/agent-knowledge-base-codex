use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn a_project_ledger_rejects_events_from_another_project_without_advancing() {
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_a).expect("open project ledger");
    let batch = EventBatch {
        source_id: "claude:foreign".to_owned(),
        events: vec![event_for(project_b)],
        quarantined: Vec::new(),
        capture_gaps: Vec::new(),
        next_cursor: SourceCursor::byte_offset(120),
    };

    let error = ledger
        .append_batch(&batch)
        .expect_err("foreign project event must fail");

    assert!(error.to_string().contains("project scope"));
    assert_eq!(ledger.event_count().expect("count events"), 0);
    assert_eq!(
        ledger.cursor("claude:foreign").expect("read cursor"),
        SourceCursor::start()
    );
}

fn event_for(project_id: ProjectId) -> NormalizedEvent {
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id: WorktreeId(uuid::Uuid::now_v7()),
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "foreign-session".to_owned(),
        native_turn_id: None,
        event_type: EventType::SessionStarted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "foreign.jsonl".to_owned(),
        source_offset: 0,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [4; 32],
        idempotency_key: [5; 32],
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({}),
        raw: serde_json::json!({"type": "session"}),
    }
}
