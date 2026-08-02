use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery, SearchSource};

#[test]
fn text_search_is_hard_scoped_and_finds_normalized_paths() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let foreign = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![event(
                event_id,
                project,
                worktree,
                "unique sentinel",
                "src/payments/router.rs",
            )],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append event");

    let hits = ledger
        .search(&SearchQuery::text(
            project,
            "payments router unique sentinel",
        ))
        .expect("search project");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source, SearchSource::Event);
    assert_eq!(hits[0].source_id, event_id);
    assert_eq!(hits[0].project_id, project);
    assert!(hits[0].bm25_score.is_finite());
    assert_eq!(hits[0].path.as_deref(), Some("src/payments/router.rs"));

    let plan = ledger
        .explain_text_search(&SearchQuery::text(project, "unique sentinel"))
        .expect("explain FTS query");
    assert!(
        plan.iter()
            .any(|detail| detail.contains("event_search") && detail.contains("VIRTUAL TABLE")),
        "unexpected FTS query plan: {plan:?}"
    );
    assert!(
        plan.iter()
            .any(|detail| detail.contains("idx_events_project_occurred")),
        "canonical candidates must begin from the project-scoped index: {plan:?}"
    );

    assert!(
        ledger
            .search(&SearchQuery::text(project, "!!!"))
            .expect("empty normalized query")
            .is_empty()
    );

    let error = ledger
        .search(&SearchQuery::text(foreign, "unique sentinel"))
        .expect_err("a ledger cannot search a foreign project");
    assert!(error.to_string().contains("project scope"));
}

fn event(
    event_id: uuid::Uuid,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    content: &str,
    path: &str,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id,
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "session-search".to_owned(),
        native_turn_id: None,
        event_type: EventType::FileModified,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH,
        observed_at: time::OffsetDateTime::UNIX_EPOCH,
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: 1,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [1; 32],
        idempotency_key: [2; 32],
        git_head: None,
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({"content": content, "path": path}),
        raw: serde_json::json!({"content": content, "path": path}),
    }
}
