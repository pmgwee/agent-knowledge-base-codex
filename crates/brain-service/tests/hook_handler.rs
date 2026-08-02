use brain_domain::{
    EventBatch, EventType, HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, NormalizedEvent,
    ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{ClaudeHookHandler, HookProjectBinding};
use brain_store::EventLedger;

#[test]
fn session_start_returns_bounded_project_scoped_context() {
    let temp = tempfile::tempdir().expect("create hook handler fixture");
    let project_root = temp.path().join("project");
    let other_root = temp.path().join("other");
    std::fs::create_dir_all(&project_root).expect("create project root");
    std::fs::create_dir_all(&other_root).expect("create foreign root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open hook ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "hook-handler-fixture".to_owned(),
            events: vec![
                event(
                    project_id,
                    worktree_id,
                    EventType::UserPrompted,
                    1,
                    "continue prior auth task",
                ),
                event(
                    project_id,
                    worktree_id,
                    EventType::TestCompleted,
                    2,
                    "auth callback tests failed",
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append hook evidence");
    drop(ledger);
    let handler = ClaudeHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path,
    })
    .expect("create hook handler");

    let reply = handler
        .handle(&envelope("SessionStart", &project_root))
        .expect("compile startup reply");
    let context = reply.additional_context.expect("startup context");
    assert!(context.contains("continue prior auth task"));
    assert!(context.contains("auth callback tests failed"));
    assert!(context.contains("Evidence:"));
    assert!(brain_context::token_count(&context) <= 1_500);

    let foreign = handler
        .handle(&envelope("SessionStart", &other_root))
        .expect("handle foreign project");
    assert_eq!(foreign.additional_context, None);
    let capture_only = handler
        .handle(&envelope("PostToolUse", &project_root))
        .expect("handle capture-only event");
    assert_eq!(capture_only.additional_context, None);
}

fn envelope(event_name: &str, cwd: &std::path::Path) -> HookEnvelope {
    HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: Harness::ClaudeCode,
        event_name: event_name.to_owned(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload: serde_json::json!({
            "hook_event_name": event_name,
            "session_id": "new-session",
            "cwd": cwd,
        }),
    }
}

fn event(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    event_type: EventType,
    sequence: i64,
    text: &str,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "prior-session".to_owned(),
        native_turn_id: Some(format!("turn-{sequence}")),
        event_type,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: sequence,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [u8::try_from(sequence).expect("small sequence"); 32],
        idempotency_key: [u8::try_from(sequence + 10).expect("small sequence"); 32],
        git_head: Some("abc123".to_owned()),
        git_branch: Some("feature/auth".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
