use brain_context::{RetrievalEngine, RetrievalQuery};
use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn exact_continuity_and_paths_are_visible_ranking_components() {
    let now = time::OffsetDateTime::from_unix_timestamp(1_775_299_200).expect("fixed now");
    let project = ProjectId(uuid::Uuid::now_v7());
    let preferred_worktree = WorktreeId(uuid::Uuid::now_v7());
    let other_worktree = WorktreeId(uuid::Uuid::now_v7());
    let task = uuid::Uuid::now_v7();
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let preferred = uuid::Uuid::now_v7();
    let other = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![
                event(
                    preferred,
                    project,
                    preferred_worktree,
                    Some(task),
                    "preferred-session",
                    "src/payments/router.rs",
                    now - time::Duration::days(2),
                    1,
                ),
                event(
                    other,
                    project,
                    other_worktree,
                    None,
                    "other-session",
                    "src/payments/router.rs",
                    now - time::Duration::hours(1),
                    2,
                ),
            ],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(2),
        })
        .expect("append events");

    let query = RetrievalQuery::text(project, "payments router", now)
        .for_worktree(preferred_worktree)
        .for_task(task)
        .for_session("preferred-session")
        .with_paths(["src/payments/router.rs"]);
    let ranked = RetrievalEngine::retrieve(&ledger, &query).expect("retrieve candidates");

    assert_eq!(ranked[0].hit.source_id, preferred);
    assert!(ranked[0].score.exact_task > 0.0);
    assert!(ranked[0].score.session_continuity > 0.0);
    assert!(ranked[0].score.worktree_match > 0.0);
    assert!(ranked[0].score.path_match > 0.0);
    assert!(ranked[0].reasons.contains(&"exact_task"));
}

#[test]
fn temporal_retrieval_preserves_late_observation_diagnostics() {
    let now = time::OffsetDateTime::from_unix_timestamp(1_775_299_200).expect("fixed now");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let event_id = uuid::Uuid::now_v7();
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let mut late = event(
        event_id,
        project,
        worktree,
        None,
        "late-session",
        "src/lib.rs",
        now - time::Duration::days(3),
        1,
    );
    late.observed_at = now;
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![late],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append event");

    let ranked = RetrievalEngine::retrieve(&ledger, &RetrievalQuery::last_week(project, now))
        .expect("retrieve last week");
    assert_eq!(ranked.len(), 1);
    assert!(ranked[0].hit.late_observation);
    assert!(ranked[0].reasons.contains(&"late_observation"));
}

#[allow(clippy::too_many_arguments)]
fn event(
    event_id: uuid::Uuid,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    task_id: Option<uuid::Uuid>,
    session: &str,
    path: &str,
    occurred_at: time::OffsetDateTime,
    sequence: u8,
) -> NormalizedEvent {
    NormalizedEvent {
        event_id,
        project_id,
        worktree_id,
        task_id,
        harness: Harness::ClaudeCode,
        native_session_id: session.to_owned(),
        native_turn_id: None,
        event_type: EventType::FileModified,
        occurred_at,
        observed_at: occurred_at,
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: i64::from(sequence),
        source_schema: "fixture:v1".to_owned(),
        raw_hash: [sequence; 32],
        idempotency_key: [sequence.saturating_add(20); 32],
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({"content": "updated payments router", "path": path}),
        raw: serde_json::json!({"content": "updated payments router", "path": path}),
    }
}
