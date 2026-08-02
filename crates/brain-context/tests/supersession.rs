use brain_context::resolve_candidates;
use brain_domain::{Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId};

#[test]
fn later_decision_supersedes_the_old_decision_without_erasing_history() {
    let project = ProjectId(uuid::Uuid::now_v7());
    let redis = decision(project, "use Redis", Vec::new());
    let sqlite = decision(project, "use SQLite", vec![redis.version_id]);

    let resolved = resolve_candidates(
        project,
        time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(1),
        vec![redis.clone(), sqlite.clone()],
    );
    assert_eq!(resolved.current.len(), 1);
    assert_eq!(resolved.current[0].content, "use SQLite");
    assert_eq!(resolved.historical.len(), 1);
    assert_eq!(resolved.historical[0].content, "use Redis");
}

#[test]
fn scope_and_validity_are_applied_before_authority() {
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let mut expired = decision(project_a, "expired A", Vec::new());
    expired.valid_to = Some(time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1));
    expired.authority = Authority::LiveState;
    let foreign = decision(project_b, "foreign B", Vec::new());
    let current = decision(project_a, "current A", Vec::new());

    let resolved = resolve_candidates(
        project_a,
        time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(2),
        vec![expired, foreign, current],
    );
    assert_eq!(resolved.current.len(), 1);
    assert_eq!(resolved.current[0].content, "current A");
    assert!(
        resolved
            .current
            .iter()
            .all(|memory| { memory.scope == MemoryScope::Project(project_a) })
    );
}

fn decision(project: ProjectId, content: &str, supersedes: Vec<uuid::Uuid>) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: None,
        task_id: None,
        kind: MemoryKind::Decision,
        title: "Storage decision".to_owned(),
        content: content.to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: vec![uuid::Uuid::now_v7()],
        supersedes,
        status: MemoryStatus::Current,
    }
}
