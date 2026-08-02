use brain_domain::{MemoryKind, ProjectId, WorktreeId};
use brain_service::{GlobalPreferenceNoteWatcher, NoteWatcher};
use brain_store::{EventLedger, GlobalPreferenceStore};

#[test]
fn project_notes_append_human_corrections_idempotently() {
    let temp = tempfile::tempdir().expect("create note fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let notes = temp.path().join("notes");
    std::fs::create_dir_all(&notes).expect("create notes");
    std::fs::write(
        notes.join("storage.md"),
        format!(
            concat!(
                "---\n",
                "title: \"Storage correction\"\n",
                "project_id: \"{}\"\n",
                "kind: decision\n",
                "evidence_ids: []\n",
                "---\n\n",
                "SQLite is the canonical store.\n"
            ),
            project.0
        ),
    )
    .expect("write note");
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let watcher = NoteWatcher::new(&notes, project, worktree);

    let first = watcher
        .scan_once(&mut ledger, time::OffsetDateTime::UNIX_EPOCH)
        .expect("first scan");
    let second = watcher
        .scan_once(&mut ledger, time::OffsetDateTime::UNIX_EPOCH)
        .expect("repeat scan");

    assert_eq!(first.imported, 1);
    assert_eq!(first.review_queued, 0);
    assert_eq!(second.unchanged, 1);
    assert_eq!(ledger.event_count().expect("event count"), 1);
    assert_eq!(ledger.memory_count().expect("memory count"), 1);
    let memory = ledger
        .current_project_memories()
        .expect("current memory")
        .pop()
        .expect("one memory");
    assert_eq!(memory.kind, MemoryKind::Decision);
    assert!(memory.content.contains("canonical store"));
    assert_eq!(memory.evidence_ids.len(), 1, "the audit event is cited");
}

#[test]
fn cross_project_or_global_promotion_notes_enter_review_without_mutation() {
    let temp = tempfile::tempdir().expect("create note fixture");
    let project = ProjectId(uuid::Uuid::now_v7());
    let foreign = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let notes = temp.path().join("notes");
    std::fs::create_dir_all(&notes).expect("create notes");
    std::fs::write(
        notes.join("foreign.md"),
        format!(
            concat!(
                "---\n",
                "title: Foreign note\n",
                "project_id: \"{}\"\n",
                "kind: preference\n",
                "promote_global: true\n",
                "---\n\n",
                "Never cross this boundary.\n"
            ),
            foreign.0
        ),
    )
    .expect("write invalid note");
    let mut ledger = EventLedger::open_in_memory(project).expect("open ledger");
    let report = NoteWatcher::new(&notes, project, worktree)
        .scan_once(&mut ledger, time::OffsetDateTime::UNIX_EPOCH)
        .expect("scan invalid note");

    assert_eq!(report.imported, 0);
    assert_eq!(report.review_queued, 1);
    assert_eq!(ledger.event_count().expect("event count"), 0);
    assert_eq!(ledger.memory_count().expect("memory count"), 0);
    assert_eq!(ledger.note_review_count().expect("review count"), 1);
}

#[test]
fn global_preferences_require_explicit_promotion_and_are_audited() {
    let temp = tempfile::tempdir().expect("create global note fixture");
    let notes = temp.path().join("notes");
    std::fs::create_dir_all(&notes).expect("create notes");
    std::fs::write(
        notes.join("style.md"),
        concat!(
            "---\n",
            "title: Response style\n",
            "kind: preference\n",
            "promote_global: true\n",
            "---\n\n",
            "Prefer concise implementation updates.\n"
        ),
    )
    .expect("write preference");
    let mut store = GlobalPreferenceStore::open_in_memory().expect("open global store");
    let watcher = GlobalPreferenceNoteWatcher::new(&notes);
    let first = watcher
        .scan_once(&mut store, time::OffsetDateTime::UNIX_EPOCH)
        .expect("import preference");
    let second = watcher
        .scan_once(&mut store, time::OffsetDateTime::UNIX_EPOCH)
        .expect("repeat preference scan");

    assert_eq!(first.imported, 1);
    assert_eq!(second.unchanged, 1);
    assert_eq!(store.preference_count().expect("preference count"), 1);
    assert_eq!(store.promotion_audit_count().expect("audit count"), 1);
}
