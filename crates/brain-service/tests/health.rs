use std::sync::Arc;

use brain_adapters::{ClaudeAdapter, NormalizeContext, SourceDescriptor};
use brain_domain::{ProjectId, WorktreeId};
use brain_service::{CaptureBinding, CaptureSupervisor, source_health_key};
use time::format_description::well_known::Rfc3339;

#[tokio::test]
async fn a_rotated_source_creates_a_visible_unresolved_capture_gap() {
    let temp = tempfile::tempdir().expect("create health fixture");
    let transcript = temp.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"future-event\",\"sessionId\":\"health-a\",\"timestamp\":\"2026-08-01T01:02:03Z\"}\n",
    )
    .expect("write first source");
    let source = SourceDescriptor::file(&transcript);
    let source_id = source.source_id.clone();
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger = temp.path().join("events.db");
    let binding = CaptureBinding::new(
        Arc::new(ClaudeAdapter::new(temp.path())),
        source,
        NormalizeContext {
            project_id,
            worktree_id,
            source_schema: "claude-jsonl:fixture".to_owned(),
        },
        &ledger,
    );
    let supervisor = CaptureSupervisor::new(vec![binding]).expect("create supervisor");
    supervisor
        .capture_once()
        .await
        .expect("capture first source");

    let initial_health = supervisor.health().expect("read initial health");
    let initial_source = initial_health
        .sources
        .get(&source_health_key(project_id, &source_id))
        .expect("initial source health");
    assert_eq!(initial_source.backlog_bytes, 0);
    assert!(initial_source.schema_fingerprint.is_some());
    let project_health = initial_health
        .projects
        .get(&project_id.0.to_string())
        .expect("project health");
    assert_eq!(project_health.persisted_events, 1);
    assert_eq!(
        project_health.last_event_at,
        Some(timestamp("2026-08-01T01:02:03Z"))
    );

    std::fs::remove_file(&transcript).expect("remove first source");
    std::fs::write(
        &transcript,
        "{\"type\":\"future-event\",\"sessionId\":\"health-b\"}\n",
    )
    .expect("write replacement source");
    supervisor
        .capture_once()
        .await
        .expect("capture replacement source");

    let health = supervisor.health().expect("read service health");
    let source_health = health
        .sources
        .get(&source_health_key(project_id, &source_id))
        .expect("source health");
    assert_eq!(source_health.capture_gaps, 1);
    assert_eq!(source_health.schema_fingerprint_changes, 1);
    assert!(source_health.last_cursor.byte_offset > 0);
    assert!(!health.is_healthy());

    drop(supervisor);
    let restarted = supervisor_for(temp.path(), &transcript, &ledger, project_id, worktree_id);
    let restarted_health = restarted.health().expect("read restarted gap health");
    let restarted_source = restarted_health
        .sources
        .get(&source_health_key(project_id, &source_id))
        .expect("restarted source health");
    assert_eq!(restarted_source.capture_gaps, 1);
    assert!(!restarted_health.is_healthy());
}

#[tokio::test]
async fn health_hydrates_persisted_cursor_and_project_totals_after_restart() {
    let temp = tempfile::tempdir().expect("create restart health fixture");
    let transcript = temp.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"future-event\",\"sessionId\":\"restart-health\",\"timestamp\":\"2026-08-01T01:02:03Z\"}\n",
    )
    .expect("write source");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger = temp.path().join("events.db");

    let first = supervisor_for(temp.path(), &transcript, &ledger, project_id, worktree_id);
    first.capture_once().await.expect("capture source");
    let expected_cursor = first
        .health()
        .expect("read first health")
        .sources
        .values()
        .next()
        .expect("first source")
        .last_cursor
        .clone();
    drop(first);

    let restarted = supervisor_for(temp.path(), &transcript, &ledger, project_id, worktree_id);
    let health = restarted.health().expect("read restarted health");
    let source = health.sources.values().next().expect("restarted source");
    assert_eq!(source.last_cursor, expected_cursor);
    let project = health
        .projects
        .get(&project_id.0.to_string())
        .expect("restarted project");
    assert_eq!(project.persisted_events, 1);
    assert_eq!(
        project.last_event_at,
        Some(timestamp("2026-08-01T01:02:03Z"))
    );
}

#[tokio::test]
async fn malformed_records_are_durably_quarantined_and_visible_after_restart() {
    let temp = tempfile::tempdir().expect("create quarantine health fixture");
    let transcript = temp.path().join("session.jsonl");
    std::fs::write(&transcript, "{ definitely-not-json }\n").expect("write malformed source");
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let ledger = temp.path().join("events.db");

    let first = supervisor_for(temp.path(), &transcript, &ledger, project_id, worktree_id);
    first
        .capture_once()
        .await
        .expect("capture malformed record");
    let first_health = first.health().expect("read quarantine health");
    let first_source = first_health.sources.values().next().expect("source health");
    assert_eq!(first_source.parse_failures, 1);
    assert_eq!(first_source.quarantined_count, 1);
    drop(first);

    let restarted = supervisor_for(temp.path(), &transcript, &ledger, project_id, worktree_id);
    let restarted_health = restarted.health().expect("read restarted health");
    let restarted_source = restarted_health
        .sources
        .values()
        .next()
        .expect("restarted source health");
    assert_eq!(restarted_source.parse_failures, 1);
    assert_eq!(restarted_source.quarantined_count, 1);
}

fn timestamp(value: &str) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(value, &Rfc3339).expect("valid fixture timestamp")
}

fn supervisor_for(
    root: &std::path::Path,
    transcript: &std::path::Path,
    ledger: &std::path::Path,
    project_id: ProjectId,
    worktree_id: WorktreeId,
) -> CaptureSupervisor {
    CaptureSupervisor::new(vec![CaptureBinding::new(
        Arc::new(ClaudeAdapter::new(root)),
        SourceDescriptor::file(transcript),
        NormalizeContext {
            project_id,
            worktree_id,
            source_schema: "claude-jsonl:fixture".to_owned(),
        },
        ledger,
    )])
    .expect("create supervisor")
}
