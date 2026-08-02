use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::EventLedger;

#[test]
fn correcting_memory_appends_a_version_and_preserves_the_old_value() {
    let project_id = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_id).expect("open ledger");
    let evidence_a = append_evidence(&mut ledger, project_id, worktree_id, 1, "use Redis");
    let evidence_b = append_evidence(&mut ledger, project_id, worktree_id, 2, "use SQLite");
    let memory_id = uuid::Uuid::now_v7();
    let first = memory(
        memory_id,
        project_id,
        worktree_id,
        "use Redis",
        evidence_a,
        vec![],
    );
    ledger.append_memory(&first).expect("append first memory");
    let mut second = memory(
        memory_id,
        project_id,
        worktree_id,
        "use SQLite",
        evidence_b,
        vec![first.version_id],
    );
    second.valid_from += time::Duration::days(30);
    ledger.append_memory(&second).expect("append correction");

    let versions = ledger.memory_versions(memory_id).expect("read versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].content, "use Redis");
    assert_eq!(versions[1].content, "use SQLite");
    assert_eq!(
        ledger
            .current_memory(memory_id)
            .expect("read current")
            .expect("current memory")
            .content,
        "use SQLite"
    );
    assert_eq!(first.projection_path(), second.projection_path());
}

#[test]
fn memory_cannot_link_evidence_from_another_project() {
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let worktree_id = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project_a).expect("open A ledger");
    let memory = memory(
        uuid::Uuid::now_v7(),
        project_b,
        worktree_id,
        "foreign project fact",
        uuid::Uuid::now_v7(),
        vec![],
    );

    let error = ledger
        .append_memory(&memory)
        .expect_err("reject foreign memory");
    assert!(error.to_string().contains("project scope"));
}

fn memory(
    id: uuid::Uuid,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    content: &str,
    evidence_id: uuid::Uuid,
    supersedes: Vec<uuid::Uuid>,
) -> MemoryRecord {
    MemoryRecord {
        id,
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project_id),
        worktree_id: Some(worktree_id),
        task_id: None,
        kind: MemoryKind::Decision,
        title: "Storage decision".to_owned(),
        content: content.to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::now_utc(),
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: vec![evidence_id],
        supersedes,
        status: MemoryStatus::Current,
    }
}

fn append_evidence(
    ledger: &mut EventLedger,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    offset: i64,
    content: &str,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id,
                worktree_id,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "fixture-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture".to_owned(),
                source_offset: offset,
                source_schema: "fixture".to_owned(),
                raw_hash: [u8::try_from(offset).expect("small offset"); 32],
                idempotency_key: [u8::try_from(offset).expect("small offset"); 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": content}),
                raw: serde_json::json!({"content": content}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(u64::try_from(offset).expect("positive")),
        })
        .expect("append evidence");
    event_id
}
