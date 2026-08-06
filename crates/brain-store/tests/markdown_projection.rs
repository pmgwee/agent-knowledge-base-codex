use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, MarkdownProjector};

#[test]
fn projection_is_deterministic_verified_and_preserves_user_notes() {
    let temp = tempfile::tempdir().expect("create vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence_id = append_evidence(&mut ledger, project, worktree);
    let memory = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: "Storage decision".to_owned(),
        content: "Use SQLite as the canonical memory store.".to_owned(),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::HumanCorrection,
        evidence_ids: vec![evidence_id],
        supersedes: Vec::new(),
        status: MemoryStatus::Current,
    };
    ledger.append_memory(&memory).expect("append memory");
    let notes = temp
        .path()
        .join("projects")
        .join(project.0.to_string())
        .join("notes");
    std::fs::create_dir_all(&notes).expect("create notes root");
    let user_note = notes.join("my-note.md");
    std::fs::write(&user_note, "# User-owned note\n").expect("write user note");

    let projector = MarkdownProjector::new(temp.path());
    let first = projector
        .rebuild_project(&ledger, project)
        .expect("first projection");
    let first_manifest = std::fs::read(&first.manifest_path).expect("read first manifest");
    let second = projector
        .rebuild_project(&ledger, project)
        .expect("second projection");
    let second_manifest = std::fs::read(&second.manifest_path).expect("read second manifest");

    assert_eq!(first.generation, second.generation);
    assert_eq!(first_manifest, second_manifest);
    assert_eq!(first.file_count, 1);
    assert!(user_note.is_file());
    let verification = projector
        .verify_project(project)
        .expect("verify projection");
    assert!(verification.valid, "{:?}", verification.errors);
    let generated = std::fs::read_to_string(&verification.files[0]).expect("read generated note");
    assert!(generated.contains(&format!("memory_id: \"{}\"", memory.id)));
    assert!(generated.contains(&format!("version_id: \"{}\"", memory.version_id)));
    assert!(generated.contains(&format!("evidence_ids: [\"{evidence_id}\"]")));
    assert!(generated.contains("- [decision] Use SQLite as the canonical memory store."));
}

#[test]
fn memories_sharing_evidence_link_to_each_other_and_never_dangle() {
    // A vault of unlinked notes is a folder, not a graph — the projection wrote 1,265 files
    // without a single wikilink between them. Edges are derived from the ledger rather than
    // proposed: two memories citing the same event were distilled from the same moment, and
    // that is a fact both notes already record in their Evidence section.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let shared = append_evidence(&mut ledger, project, worktree);

    let first = memory(
        project,
        worktree,
        "Bounded the job window",
        vec![shared],
        Vec::new(),
    );
    let second = memory(
        project,
        worktree,
        "Counted raw bytes as well as payload",
        vec![shared],
        Vec::new(),
    );
    ledger.append_memory(&first).expect("append first");
    ledger.append_memory(&second).expect("append second");

    let projector = MarkdownProjector::new(temp.path());
    projector
        .rebuild_project(&ledger, project)
        .expect("projection");
    let verification = projector.verify_project(project).expect("verify");
    assert!(verification.valid, "{:?}", verification.errors);

    let notes: Vec<String> = verification
        .files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read note"))
        .collect();
    let joined = notes.join("\n");

    assert!(
        joined.contains(&format!("[[{}|Bounded the job window]]", first.id)),
        "each note must link to the other by id, aliased to its title"
    );
    assert!(
        joined.contains(&format!(
            "[[{}|Counted raw bytes as well as payload]]",
            second.id
        )),
        "the reciprocal link must exist too"
    );
    assert!(
        joined.contains("## Related"),
        "linked notes must carry a Related section"
    );
    assert!(
        !joined.contains("## Supersedes"),
        "neither memory supersedes anything, so that section must be absent"
    );
}

#[test]
fn a_link_to_a_superseded_memory_is_dropped_rather_than_left_dangling() {
    // The projection renders only *current* memories, so a note that supersedes an older one
    // names a memory the vault does not contain. Writing that link anyway would invite a reader
    // — and Obsidian's graph — to chase a note that was deliberately not published.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);

    let old = memory(
        project,
        worktree,
        "Bound by count only",
        vec![evidence],
        Vec::new(),
    );
    ledger.append_memory(&old).expect("append old");
    let replacement = memory(
        project,
        worktree,
        "Bound by count and bytes",
        vec![evidence],
        vec![old.version_id],
    );
    ledger
        .append_memory(&replacement)
        .expect("append replacement");

    let projector = MarkdownProjector::new(temp.path());
    projector
        .rebuild_project(&ledger, project)
        .expect("project");
    let verification = projector.verify_project(project).expect("verify");
    let joined = verification
        .files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !joined.contains(&format!("[[{}|", old.version_id)),
        "a wikilink to a memory absent from the projection must not be written"
    );
}

#[test]
fn a_memory_that_shares_nothing_gets_no_empty_link_sections() {
    // An empty "Related" heading reads as "checked, and there are none". Usually it means this
    // is simply the only memory citing its evidence so far, which is not the same claim.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);
    let only = memory(
        project,
        worktree,
        "Stands alone",
        vec![evidence],
        Vec::new(),
    );
    ledger.append_memory(&only).expect("append");

    let projector = MarkdownProjector::new(temp.path());
    projector
        .rebuild_project(&ledger, project)
        .expect("project");
    let verification = projector.verify_project(project).expect("verify");
    let note = std::fs::read_to_string(&verification.files[0]).expect("read");

    assert!(!note.contains("## Related"));
    assert!(!note.contains("## Supersedes"));
    assert!(note.contains("## Evidence"), "citations still belong there");
}

fn memory(
    project: ProjectId,
    worktree: WorktreeId,
    title: &str,
    evidence_ids: Vec<uuid::Uuid>,
    supersedes: Vec<uuid::Uuid>,
) -> MemoryRecord {
    MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(project),
        worktree_id: Some(worktree),
        task_id: None,
        kind: MemoryKind::Decision,
        title: title.to_owned(),
        content: format!("{title}."),
        valid_from: time::OffsetDateTime::UNIX_EPOCH,
        valid_to: None,
        recorded_at: time::OffsetDateTime::UNIX_EPOCH,
        confidence: 1.0,
        authority: Authority::DerivedMemory,
        evidence_ids,
        supersedes,
        status: MemoryStatus::Current,
    }
}

fn append_evidence(
    ledger: &mut EventLedger,
    project: ProjectId,
    worktree: WorktreeId,
) -> uuid::Uuid {
    let event_id = uuid::Uuid::now_v7();
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::ClaudeCode,
                native_session_id: "fixture-session".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: time::OffsetDateTime::UNIX_EPOCH,
                observed_at: time::OffsetDateTime::UNIX_EPOCH,
                source_locator: "fixture.jsonl".to_owned(),
                source_offset: 1,
                source_schema: "fixture:v1".to_owned(),
                raw_hash: [1; 32],
                idempotency_key: [2; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"content": "Choose SQLite"}),
                raw: serde_json::json!({"content": "Choose SQLite"}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append evidence");
    event_id
}
