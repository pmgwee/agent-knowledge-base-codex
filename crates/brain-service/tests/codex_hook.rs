use brain_domain::{
    EventBatch, EventType, HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, NormalizedEvent,
    ProjectId, SourceCursor, WorktreeId,
};
use brain_service::{HookProjectBinding, ProjectHookHandler};
use brain_store::EventLedger;

#[test]
fn codex_session_start_uses_the_shared_bounded_project_context() {
    let temp = tempfile::tempdir().expect("create Codex hook fixture");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger_path = temp.path().join("events.db");
    let mut ledger = EventLedger::open(&ledger_path, project_id).expect("open ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "prior-claude-session".to_owned(),
            events: vec![
                event(
                    project_id,
                    worktree_id,
                    EventType::UserPrompted,
                    1,
                    "implement auth callback",
                ),
                event(
                    project_id,
                    worktree_id,
                    EventType::AgentResponded,
                    2,
                    "tests failed",
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append prior evidence");
    drop(ledger);
    let handler = ProjectHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id,
        worktree_id,
        ledger_path,
        global_preferences_path: None,
        brain_home: std::path::PathBuf::new(),
    })
    .expect("create shared hook handler");

    let mut hook_input: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/codex/hook-input.json"))
            .expect("parse reviewed Codex hook fixture");
    hook_input["cwd"] = serde_json::json!(project_root);
    let reply = handler
        .handle(&HookEnvelope {
            protocol: HOOK_PROTOCOL_VERSION,
            harness: Harness::Codex,
            event_name: "SessionStart".to_owned(),
            received_at: time::OffsetDateTime::now_utc(),
            nonce: uuid::Uuid::now_v7(),
            payload: hook_input,
        })
        .expect("compile Codex startup context")
        .reply;
    let context = reply.additional_context.expect("Codex additional context");

    assert!(context.contains("implement auth callback"));
    assert!(context.contains("tests failed"));
    assert!(context.contains("Evidence: event:"));
    assert!(brain_context::token_count(&context) <= 1_500);
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
        native_session_id: "claude-prior-session".to_owned(),
        native_turn_id: Some(format!("turn-{sequence}")),
        event_type,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        source_locator: "claude.jsonl".to_owned(),
        source_offset: sequence,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [u8::try_from(sequence).expect("small sequence"); 32],
        idempotency_key: [u8::try_from(sequence + 20).expect("small sequence"); 32],
        git_head: None,
        git_branch: Some("feature/auth".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
