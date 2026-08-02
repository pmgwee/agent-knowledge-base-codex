use brain_context::{ContextCompiler, ContextEvidence, ContextQuery, token_count};
use brain_domain::{EventType, Harness, ProjectId, WorktreeId};

#[test]
fn orientation_never_crosses_1500_tokens_and_cites_every_included_fact() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let mut events = vec![
        evidence(
            project_id,
            worktree_id,
            EventType::UserPrompted,
            1_000,
            &format!("implement bounded task: {}", "task();".repeat(200)),
        ),
        evidence(
            project_id,
            worktree_id,
            EventType::AgentResponded,
            1_001,
            &format!("latest outcome: {}", "outcome();".repeat(200)),
        ),
    ];
    for sequence in 0..100 {
        let event_type = if sequence % 2 == 0 {
            EventType::ToolCompleted
        } else {
            EventType::FileModified
        };
        events.push(evidence(
            project_id,
            worktree_id,
            event_type,
            sequence,
            &format!("low priority {sequence}: {}", "detail();".repeat(200)),
        ));
    }
    let compiler = ContextCompiler::from_events(events);

    let result = compiler
        .compile(ContextQuery::for_worktree(project_id, worktree_id))
        .expect("compile bounded orientation");

    assert!(result.token_count <= 1500);
    assert_eq!(result.token_count, token_count(&result.text));
    assert!(result.text.contains("Evidence:"));
    assert!(!result.evidence_ids.is_empty());
    assert!(
        result.evidence_ids.len() < 11,
        "whole low-priority blocks should be dropped"
    );
    for evidence_id in &result.evidence_ids {
        assert!(result.text.contains(&evidence_id.to_string()));
    }
}

#[test]
fn another_project_cannot_enter_orientation_even_in_a_mixed_candidate_set() {
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let compiler = ContextCompiler::from_events(vec![
        evidence(
            project_a,
            worktree,
            EventType::UserPrompted,
            2,
            "PROJECT_A_TASK",
        ),
        evidence(
            project_b,
            worktree,
            EventType::UserPrompted,
            3,
            "PROJECT_B_SENTINEL",
        ),
    ]);

    let result = compiler
        .compile(ContextQuery::for_worktree(project_a, worktree))
        .expect("compile project A orientation");

    assert!(result.text.contains("PROJECT_A_TASK"));
    assert!(!result.text.contains("PROJECT_B_SENTINEL"));
}

#[test]
fn orientation_prefers_latest_task_outcome_test_and_revision_evidence() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut task = evidence(
        project,
        worktree,
        EventType::UserPrompted,
        10,
        "implement auth callback",
    );
    task.git_branch = Some("feature/auth".to_owned());
    task.git_head = Some("abc123".to_owned());
    let compiler = ContextCompiler::from_events(vec![
        evidence(
            project,
            worktree,
            EventType::UserPrompted,
            1,
            "obsolete task",
        ),
        task,
        evidence(
            project,
            worktree,
            EventType::AgentResponded,
            11,
            "callback implemented; review redirect validation",
        ),
        evidence(
            project,
            worktree,
            EventType::TestCompleted,
            12,
            "auth callback test failed",
        ),
    ]);

    let result = compiler
        .compile(ContextQuery::for_worktree(project, worktree))
        .expect("compile relevant orientation");

    assert!(result.text.contains("implement auth callback"));
    assert!(!result.text.contains("obsolete task"));
    assert!(result.text.contains("callback implemented"));
    assert!(result.text.contains("auth callback test failed"));
    assert!(result.text.contains("feature/auth"));
    assert!(result.text.contains("abc123"));
}

#[test]
fn empty_or_unusable_history_never_fabricates_context() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut corrupt_shape = evidence(project, worktree, EventType::SchemaUnknown, 1, "ignored");
    corrupt_shape.payload = serde_json::Value::Null;
    corrupt_shape.raw = serde_json::Value::Null;

    for compiler in [
        ContextCompiler::from_events(Vec::new()),
        ContextCompiler::from_events(vec![corrupt_shape]),
    ] {
        let result = compiler
            .compile(ContextQuery::for_worktree(project, worktree))
            .expect("compile no-history orientation");
        assert!(result.text.contains("No prior project evidence"));
        assert!(result.evidence_ids.is_empty());
    }
}

fn evidence(
    project_id: ProjectId,
    worktree_id: WorktreeId,
    event_type: EventType,
    sequence: i64,
    text: &str,
) -> ContextEvidence {
    ContextEvidence {
        event_id: uuid::Uuid::now_v7(),
        project_id,
        worktree_id,
        task_id: None,
        harness: Harness::ClaudeCode,
        native_session_id: "prior-session".to_owned(),
        event_type,
        occurred_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        observed_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(sequence),
        source_offset: sequence,
        git_head: None,
        git_branch: None,
        payload: serde_json::json!({"text": text}),
        raw: serde_json::json!({"text": text}),
    }
}
