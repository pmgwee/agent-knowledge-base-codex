use brain_context::{ContextCompiler, ContextQuery};
use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn compiler_loads_only_a_bounded_hard_scoped_ledger_slice() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let other_project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open context ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "context-fixture".to_owned(),
            events: vec![event(project, worktree, "STORED_PROJECT_TASK")],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append context evidence");

    let compiler =
        ContextCompiler::from_ledger(&ledger, project, 200).expect("load bounded ledger events");
    let context = compiler
        .compile(ContextQuery::for_worktree(project, worktree))
        .expect("compile stored context");

    assert!(context.text.contains("STORED_PROJECT_TASK"));
    assert!(
        ContextCompiler::from_ledger(&ledger, other_project, 200).is_err(),
        "a project-scoped ledger must reject a foreign query before reading"
    );
}

fn event(project_id: ProjectId, worktree_id: WorktreeId, text: &str) -> NormalizedEvent {
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "stored-session".to_owned(),
        native_turn_id: Some("stored-turn".to_owned()),
        event_type: EventType::UserPrompted,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(2),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: 1,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [1; 32],
        idempotency_key: [2; 32],
        git_head: Some("abc123".to_owned()),
        git_branch: Some("feature/context".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
