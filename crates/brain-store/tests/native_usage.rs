use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;
use sha2::{Digest, Sha256};

#[test]
fn native_usage_query_is_time_bounded_and_project_scoped() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let other = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(500);
    let mut ledger = EventLedger::open_in_memory(project).unwrap();
    ledger
        .append_batch(&EventBatch {
            source_id: "native-usage".to_owned(),
            events: vec![
                event(project, worktree, Harness::ClaudeCode, now, 1),
                event(
                    project,
                    worktree,
                    Harness::Codex,
                    now + time::Duration::seconds(1),
                    2,
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .unwrap();
    let rows = ledger
        .native_usage_events(project, now - time::Duration::seconds(1))
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].native_session_id, "session-1");
    assert!(ledger.native_usage_events(other, now).is_err());
    assert!(
        ledger
            .native_usage_events(project, now + time::Duration::hours(1))
            .unwrap()
            .is_empty()
    );
}

fn event(
    project: ProjectId,
    worktree: WorktreeId,
    harness: Harness,
    now: time::OffsetDateTime,
    index: i64,
) -> NormalizedEvent {
    let raw = match harness {
        Harness::ClaudeCode => serde_json::json!({
            "message": {"id": format!("message-{index}")},
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": 2,
                "cache_read_input_tokens": 3,
                "output_tokens": 4
            }
        }),
        Harness::Codex => serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {
                "input_tokens": 10, "output_tokens": 4, "total_tokens": 14
            }}}
        }),
        _ => unreachable!(),
    };
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness,
        native_session_id: format!("session-{index}"),
        native_turn_id: Some(index.to_string()),
        event_type: EventType::AgentResponded,
        occurred_at: now + time::Duration::seconds(index),
        observed_at: now + time::Duration::seconds(index),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: index,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: Sha256::digest(serde_json::to_vec(&raw).unwrap()).into(),
        idempotency_key: Sha256::digest(format!("native:{index}").as_bytes()).into(),
        git_head: None,
        git_branch: None,
        payload: raw.clone(),
        raw,
    }
}
