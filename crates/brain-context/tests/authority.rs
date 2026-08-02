use brain_context::resolve_candidates;
use brain_domain::{Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId};

#[test]
fn live_test_failure_overrides_an_old_passing_checkpoint() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let old = memory(
        project,
        "Test status",
        "tests passing",
        Authority::AgentCheckpoint,
        1,
    );
    let live = memory(
        project,
        "Test status",
        "tests currently failing",
        Authority::LiveState,
        2,
    );

    let resolved = resolve_candidates(
        project,
        time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(3),
        vec![old, live],
    );
    assert_eq!(resolved.current.len(), 1);
    assert_eq!(resolved.current[0].authority, Authority::LiveState);
    assert!(resolved.current[0].content.contains("currently failing"));
    assert!(resolved.rendered_warning().contains("tests passing"));
}

#[test]
fn equal_authority_contradictions_remain_a_visible_conflict() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let redis = memory(
        project,
        "Cache backend",
        "Redis",
        Authority::HumanCorrection,
        1,
    );
    let sqlite = memory(
        project,
        "Cache backend",
        "SQLite",
        Authority::HumanCorrection,
        2,
    );

    let resolved = resolve_candidates(
        project,
        time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(3),
        vec![redis, sqlite],
    );
    assert_eq!(resolved.conflicts.len(), 1);
    assert_eq!(resolved.conflicts[0].records.len(), 2);
    assert!(resolved.rendered_warning().contains("unresolved conflict"));
}

fn memory(
    project: ProjectId,
    title: &str,
    content: &str,
    authority: Authority,
    hour: i64,
) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: None,
        task_id: None,
        kind: MemoryKind::Fact,
        title: title.to_owned(),
        content: content.to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(hour),
        confidence: 1.0,
        authority,
        evidence_ids: vec![uuid::Uuid::now_v7()],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    }
}
