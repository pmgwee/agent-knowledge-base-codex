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
    // The memory, plus the generated project index.
    assert_eq!(first.file_count, 2);
    assert!(user_note.is_file());
    let verification = projector
        .verify_project(project)
        .expect("verify projection");
    assert!(verification.valid, "{:?}", verification.errors);
    let generated = read_note_containing(&verification.files, &memory.id.to_string());
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
        joined.contains("|Bounded the job window]]"),
        "each note must link to the other, aliased to its title"
    );
    assert!(
        joined.contains("|Counted raw bytes as well as payload]]"),
        "the reciprocal link must exist too"
    );
    assert!(
        joined.contains("[[bounded-the-job-window-"),
        "links resolve by filename, which is the title slug"
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

#[test]
fn the_index_lists_every_memory_and_asserts_nothing_of_its_own() {
    // Peer links alone make a graph you can only enter if you already know a note. The index is
    // the front door — and it must stay pure navigation, so it cannot contradict the notes it
    // lists.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);
    let one = memory(
        project,
        worktree,
        "First decision",
        vec![evidence],
        Vec::new(),
    );
    let two = memory(
        project,
        worktree,
        "Second decision",
        vec![evidence],
        Vec::new(),
    );
    ledger.append_memory(&one).expect("append one");
    ledger.append_memory(&two).expect("append two");

    let projector = MarkdownProjector::new(temp.path());
    projector
        .rebuild_project(&ledger, project)
        .expect("project");
    let verification = projector.verify_project(project).expect("verify");
    assert!(verification.valid, "{:?}", verification.errors);

    let index = read_note_containing(&verification.files, "Project memory index");
    assert!(
        index.contains("|First decision]]"),
        "the index must link every memory"
    );
    assert!(index.contains("|Second decision]]"));
    assert!(
        index.contains("[[first-decision-"),
        "index links resolve by filename too"
    );
    assert!(
        index.contains("2 memories"),
        "the total must be stated so a truncated list never understates the vault"
    );
    assert!(
        !index.contains("## Evidence"),
        "the index carries no claims, so it cites nothing"
    );
}

#[test]
fn notes_are_named_after_their_titles_so_the_graph_is_readable() {
    // Obsidian labels every graph node with the filename — not the `title` front-matter, not
    // the alias in `[[id|Title]]`. Naming notes by memory id produced a graph of several
    // hundred UUIDs: structurally correct, completely unreadable.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);
    let one = memory(
        project,
        worktree,
        "Bound the job window by bytes",
        vec![evidence],
        Vec::new(),
    );
    let two = memory(
        project,
        worktree,
        "Counted raw as well as payload",
        vec![evidence],
        Vec::new(),
    );
    ledger.append_memory(&one).expect("append one");
    ledger.append_memory(&two).expect("append two");

    let projector = MarkdownProjector::new(temp.path());
    projector
        .rebuild_project(&ledger, project)
        .expect("project");
    let verification = projector.verify_project(project).expect("verify");

    let names: Vec<String> = verification
        .files
        .iter()
        .map(|path| path.file_name().expect("named").to_string_lossy().into())
        .collect();
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("bound-the-job-window-by-bytes-")),
        "a note must be named after its title, got {names:?}"
    );
    assert!(
        names.iter().all(|name| name.ends_with(".md")),
        "got {names:?}"
    );

    // The links have to follow the filenames, or every one of them dangles.
    let joined = verification
        .files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("[[bound-the-job-window-by-bytes-"),
        "wikilinks must target the note's filename, not its id"
    );
    assert!(
        !joined.contains(&format!("[[{}|", one.id)),
        "a link built from the id no longer resolves and must not be written"
    );
}

#[test]
fn superseded_generations_are_pruned_so_the_vault_holds_one_copy() {
    // Generations are content-addressed, so every rebuild that changes anything leaves the old
    // directory behind. Nothing collected them: the live vault reached 110 stale generations
    // holding 10,911 files against 558 current ones, and Obsidian showed the same notes twenty
    // times over as disconnected islands.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);

    let first = memory(project, worktree, "First", vec![evidence], Vec::new());
    ledger.append_memory(&first).expect("append first");
    let projector = MarkdownProjector::new(temp.path());
    let one = projector.rebuild_project(&ledger, project).expect("first");

    // A second memory changes the content, so a new generation is published.
    let second = memory(project, worktree, "Second", vec![evidence], Vec::new());
    ledger.append_memory(&second).expect("append second");
    let two = projector.rebuild_project(&ledger, project).expect("second");
    assert_ne!(one.generation, two.generation, "content changed");

    let generations = temp
        .path()
        .join("projects")
        .join(project.0.to_string())
        .join("generated")
        .join("generations");
    let kept: Vec<String> = std::fs::read_dir(&generations)
        .expect("list generations")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into())
        .collect();
    assert_eq!(
        kept,
        vec![two.generation.clone()],
        "only the published generation survives"
    );
    assert_eq!(two.pruned_generations, 1);

    // And the vault still verifies against what remains.
    assert!(
        projector.verify_project(project).expect("verify").valid,
        "pruning must not break the published generation"
    );
}

#[test]
fn an_abandoned_staging_directory_is_collected_but_a_live_one_is_left_alone() {
    // The other half of the same leak. A pass writes into `staging-<uuid>` and renames it into
    // place, so a process that stops in between leaves the directory behind — and skipping every
    // `staging-*` meant nothing ever collected it. Deploys restart this service routinely; the
    // live vault reached 5,834 orphaned files across two projects, each a duplicate in Obsidian.
    //
    // Age is the only thing that separates a dead staging directory from one being written right
    // now, so both directions are pinned here: deleting a live one would corrupt a publish, and
    // that is the more expensive mistake.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);
    let record = memory(project, worktree, "Only", vec![evidence], Vec::new());
    ledger.append_memory(&record).expect("append");

    let projector = MarkdownProjector::new(temp.path());
    projector.rebuild_project(&ledger, project).expect("first");

    let generations = temp
        .path()
        .join("projects")
        .join(project.0.to_string())
        .join("generated")
        .join("generations");

    // One staging directory left behind by a killed pass, and one a pass is writing right now.
    // The age is in the name — a v7 UUID carries the millisecond it was minted — so this needs no
    // clock games on the filesystem. The stale id below is a real one recovered from the live
    // vault; the live id is minted here and is therefore seconds old.
    let abandoned = generations.join("staging-019fd80e-c8dd-7f92-8ac4-066bda3dab87");
    let live = generations.join(format!("staging-{}", uuid::Uuid::now_v7()));
    for path in [&abandoned, &live] {
        std::fs::create_dir_all(path).expect("create staging");
        std::fs::write(path.join("note.md"), "orphan").expect("write");
    }

    // A content change publishes a new generation, which is what runs the pruner.
    let second = memory(project, worktree, "Second", vec![evidence], Vec::new());
    ledger.append_memory(&second).expect("append second");
    projector
        .rebuild_project(&ledger, project)
        .expect("second rebuild");

    assert!(
        !abandoned.exists(),
        "a staging directory untouched for hours is not one anybody is still writing"
    );
    assert!(
        live.exists(),
        "a staging directory being written must survive; deleting it corrupts a publish"
    );
    assert!(
        projector.verify_project(project).expect("verify").valid,
        "collecting staging must not disturb the published generation"
    );
}

#[test]
fn the_vault_keeps_a_greppable_chronological_log() {
    // The generated tree answers "what does this project know" and cannot answer "what happened,
    // and when" — a content-addressed generation replaces its predecessor, so the vault has no
    // history of its own even though the ledger underneath it is nothing but history.
    let temp = tempfile::tempdir().expect("vault fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let evidence = append_evidence(&mut ledger, project, worktree);
    let record = memory(project, worktree, "First", vec![evidence], Vec::new());
    ledger.append_memory(&record).expect("append");

    let projector = MarkdownProjector::new(temp.path());
    projector.rebuild_project(&ledger, project).expect("first");
    let second = memory(project, worktree, "Second", vec![evidence], Vec::new());
    ledger.append_memory(&second).expect("append second");
    projector.rebuild_project(&ledger, project).expect("second");

    let log = std::fs::read_to_string(
        temp.path()
            .join("projects")
            .join(project.0.to_string())
            .join("log.md"),
    )
    .expect("read log");

    // The prefix is the whole query language: `grep "^## \[" log.md | tail -5`.
    let entries: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("## ["))
        .collect();
    assert_eq!(entries.len(), 2, "one line per projection, appended: {log}");
    assert!(entries[0].contains("projection |"), "{}", entries[0]);
    assert!(
        entries[1].contains("2 notes") || entries[1].contains("3 notes"),
        "the line states what was written, got {}",
        entries[1]
    );
}

fn read_note_containing(files: &[std::path::PathBuf], needle: &str) -> String {
    files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read note"))
        .find(|text| text.contains(needle))
        .unwrap_or_else(|| panic!("no generated note contains {needle:?}"))
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
