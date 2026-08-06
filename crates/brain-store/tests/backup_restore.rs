use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{BackupManager, EventLedger, RetentionPolicy};
use sha2::{Digest, Sha256};

#[test]
fn online_backup_restores_a_consistent_ledger_and_complete_inventory() {
    let fixture = fixture();
    let backup = BackupManager::create(
        &fixture.brain_home,
        fixture.temp.path().join("backups"),
        fixture.now,
    )
    .expect("backup");
    let verification = BackupManager::verify(&backup.backup_path).expect("verify");
    assert_eq!(verification.sqlite_integrity_checks, 1);
    let restored = BackupManager::restore_isolated(
        &backup.backup_path,
        fixture.temp.path().join("restored brain"),
    )
    .expect("restore");
    let ledger = EventLedger::open(
        restored.destination.join("projects/p1/ledger.sqlite"),
        fixture.project,
    )
    .expect("restored ledger");
    assert_eq!(ledger.event_count().expect("event count"), 3);
    assert_eq!(
        std::fs::read_to_string(restored.destination.join("config.json")).expect("config"),
        "{\"schema\":1}\n"
    );
}

#[test]
fn checksum_failure_never_creates_or_replaces_a_restore_destination() {
    let fixture = fixture();
    let backup = BackupManager::create(
        &fixture.brain_home,
        fixture.temp.path().join("backups"),
        fixture.now,
    )
    .expect("backup");
    std::fs::write(backup.backup_path.join("config.json"), "corrupt\n").expect("corrupt");
    let destination = fixture.temp.path().join("must not exist");
    assert!(BackupManager::restore_isolated(&backup.backup_path, &destination).is_err());
    assert!(!destination.exists());
    assert_eq!(
        EventLedger::open(&fixture.ledger_path, fixture.project)
            .expect("source ledger")
            .event_count()
            .expect("source count"),
        3
    );
}

#[test]
fn retention_is_deterministic_dry_runnable_and_ignores_unknown_directories() {
    let fixture = fixture();
    let backups = fixture.temp.path().join("backups");
    let first =
        BackupManager::create(&fixture.brain_home, &backups, fixture.now).expect("first backup");
    let second = BackupManager::create(
        &fixture.brain_home,
        &backups,
        fixture.now + time::Duration::hours(1),
    )
    .expect("second backup");
    let third = BackupManager::create(
        &fixture.brain_home,
        &backups,
        fixture.now + time::Duration::hours(2),
    )
    .expect("third backup");
    let unknown = backups.join("operator-notes");
    std::fs::create_dir_all(&unknown).expect("unknown directory");
    let policy = RetentionPolicy {
        hourly: 2,
        daily: 1,
        monthly: 1,
    };
    let dry_run = BackupManager::apply_retention(&backups, policy, true).expect("dry run");
    assert_eq!(dry_run.retained.len(), 2);
    assert_eq!(dry_run.pruned, vec![first.backup_path.clone()]);
    assert!(
        dry_run
            .ignored
            .iter()
            .any(|path| path.file_name() == unknown.file_name())
    );
    assert!(first.backup_path.is_dir());

    let applied = BackupManager::apply_retention(&backups, policy, false).expect("apply retention");
    assert_eq!(applied.pruned, vec![first.backup_path.clone()]);
    assert!(!first.backup_path.exists());
    assert!(second.backup_path.is_dir());
    assert!(third.backup_path.is_dir());
    assert!(unknown.is_dir());
}

#[test]
fn recovery_drill_records_success_or_failure_without_leaving_a_restore_copy() {
    let fixture = fixture();
    let backup = BackupManager::create(
        &fixture.brain_home,
        fixture.temp.path().join("backups"),
        fixture.now,
    )
    .expect("backup");
    let drill_root = fixture.temp.path().join("drills");
    let successful = BackupManager::recovery_drill(
        &backup.backup_path,
        &drill_root,
        fixture.now + time::Duration::hours(1),
    )
    .expect("successful drill");
    assert!(successful.success);
    assert_eq!(
        successful.restored_files,
        backup.inventory.files.len() as u64
    );
    assert!(successful.report_path.is_file());
    assert_eq!(
        std::fs::read_dir(&drill_root)
            .expect("list drill root")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("restore-"))
            .count(),
        0
    );

    std::fs::write(backup.backup_path.join("config.json"), "corrupt\n").expect("corrupt backup");
    let failed = BackupManager::recovery_drill(
        &backup.backup_path,
        &drill_root,
        fixture.now + time::Duration::hours(2),
    )
    .expect("failed drill is recorded");
    assert!(!failed.success);
    assert!(failed.error.is_some());
    assert!(failed.report_path.is_file());
}

#[test]
fn backups_skip_rebuildable_binaries_but_keep_everything_else() {
    // Binaries are build output: reproducible from the commit the deploy manifest records and
    // hash-verified there. Copying them into all 66 retained snapshots spends gigabytes to
    // protect something one command regenerates. Everything that is *not* rebuildable must
    // still be captured, which is the half of this worth guarding against an over-broad rule.
    let fixture = fixture();
    std::fs::create_dir_all(fixture.brain_home.join("bin")).expect("bin");
    std::fs::write(fixture.brain_home.join("bin/brain.exe"), b"MZ fake binary").expect("exe");
    std::fs::write(fixture.brain_home.join("runtime.json"), "{}\n").expect("runtime");

    let backup = BackupManager::create(
        &fixture.brain_home,
        fixture.temp.path().join("backups"),
        fixture.now,
    )
    .expect("backup");

    let captured: Vec<String> = backup
        .inventory
        .files
        .iter()
        .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
        .collect();

    assert!(
        !captured.iter().any(|path| path.starts_with("bin/")),
        "bin/ must not be captured, got {captured:?}"
    );
    assert!(
        captured.iter().any(|path| path == "runtime.json"),
        "non-rebuildable files must still be captured, got {captured:?}"
    );
    assert!(
        captured.iter().any(|path| path.ends_with("ledger.sqlite")),
        "the ledger is the whole point of the backup, got {captured:?}"
    );

    // The inventory hash covers only what was captured, so verification must still pass.
    BackupManager::verify(&backup.backup_path).expect("verify");
    let restored =
        BackupManager::restore_isolated(&backup.backup_path, fixture.temp.path().join("restored"))
            .expect("restore");
    assert!(
        !restored.destination.join("bin").exists(),
        "a restore reinstates evidence; binaries come from a deploy"
    );
}

struct Fixture {
    temp: tempfile::TempDir,
    brain_home: std::path::PathBuf,
    ledger_path: std::path::PathBuf,
    project: ProjectId,
    now: time::OffsetDateTime,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let ledger_path = brain_home.join("projects/p1/ledger.sqlite");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let mut ledger = EventLedger::open(&ledger_path, project).expect("ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: (0..3)
                .map(|index| event(project, worktree, now, index))
                .collect(),
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(3),
        })
        .expect("append");
    std::fs::create_dir_all(&brain_home).expect("brain home");
    std::fs::write(brain_home.join("config.json"), "{\"schema\":1}\n").expect("config");
    Fixture {
        temp,
        brain_home,
        ledger_path,
        project,
        now,
    }
}

fn event(
    project: ProjectId,
    worktree: WorktreeId,
    now: time::OffsetDateTime,
    index: i64,
) -> NormalizedEvent {
    let raw = serde_json::json!({"index": index});
    NormalizedEvent {
        event_id: uuid::Uuid::now_v7(),
        project_id: project,
        worktree_id: worktree,
        task_id: None,
        harness: Harness::Codex,
        native_session_id: "backup-fixture".to_owned(),
        native_turn_id: Some(index.to_string()),
        event_type: EventType::AgentResponded,
        occurred_at: now + time::Duration::seconds(index),
        observed_at: now + time::Duration::seconds(index),
        source_locator: "fixture.jsonl".to_owned(),
        source_offset: index,
        source_schema: "fixture:v1".to_owned(),
        raw_hash: Sha256::digest(serde_json::to_vec(&raw).expect("raw")).into(),
        idempotency_key: Sha256::digest(format!("backup:{index}").as_bytes()).into(),
        git_head: None,
        git_branch: None,
        payload: raw.clone(),
        raw,
    }
}
