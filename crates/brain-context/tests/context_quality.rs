use brain_context::{ContextCompiler, ContextEvidence, ContextQuery, LiveState, ProviderResult};
use brain_domain::{
    Authority, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId,
    WorktreeId,
};

#[test]
fn current_compatible_failure_suppresses_stored_passing_claim() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let failure = evidence(
        project,
        worktree,
        EventType::TestCompleted,
        "tests currently failing: auth callback",
        "new-head",
        20,
    );
    let compiler = ContextCompiler::from_events(vec![failure])
        .with_memories(vec![memory(
            project,
            "Test status",
            "Current status: passing",
            Authority::AgentCheckpoint,
        )])
        .with_live_state(LiveState::fixture_clean(worktree, "main", "new-head"));

    let context = compiler
        .compile(ContextQuery::for_worktree(project, worktree))
        .expect("compile context");
    assert!(context.text.contains("tests currently failing"));
    assert!(!context.text.contains("Current status: passing"));
    assert!(context.text.contains("event:"));
}

#[test]
fn hard_budget_survives_oversized_optional_provider_results() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let events = (0..50)
        .map(|sequence| {
            evidence(
                project,
                worktree,
                EventType::AgentResponded,
                &format!("outcome {sequence} {}", "large ".repeat(500)),
                "head",
                sequence,
            )
        })
        .collect();
    let providers = (0..20)
        .map(|sequence| ProviderResult {
            provider: "fixture".to_owned(),
            title: format!("provider result {sequence}"),
            content: "provider content ".repeat(1_000),
            source_uri: format!("fixture://result/{sequence}"),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            trust: "external_document".to_owned(),
        })
        .collect();
    let compiler = ContextCompiler::from_events(events).with_provider_results(providers);
    let mut query = ContextQuery::for_worktree(project, worktree);
    query.max_tokens = 3_000;
    let context = compiler.compile(query).expect("compile bounded context");
    assert!(context.token_count <= 3_000);
    assert_eq!(
        context.token_count,
        brain_context::token_count(&context.text)
    );
}

fn evidence(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    event_type: EventType,
    text: &str,
    git_head: &str,
    sequence: i64,
) -> ContextEvidence {
    ContextEvidence {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::Codex,
        native_session_id: "session".to_owned(),
        event_type,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        source_offset: sequence,
        git_head: Some(git_head.to_owned()),
        git_branch: Some("main".to_owned()),
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}

fn memory(project: ProjectId, title: &str, content: &str, authority: Authority) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: None,
        task_id: None,
        kind: MemoryKind::Checkpoint,
        title: title.to_owned(),
        content: content.to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority,
        evidence_ids: vec![uuid::Uuid::now_v7()],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
