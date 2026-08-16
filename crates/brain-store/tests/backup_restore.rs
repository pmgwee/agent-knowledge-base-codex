use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{ABANDONED_STAGING_AGE, BackupManager, EventLedger, RetentionPolicy};
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
    let dry_run = BackupManager::apply_retention(&backups, policy, ABANDONED_STAGING_AGE, true)
        .expect("dry run");
    assert_eq!(dry_run.retained.len(), 2);
    assert_eq!(dry_run.pruned, vec![first.backup_path.clone()]);
    assert!(
        dry_run
            .ignored
            .iter()
            .any(|path| path.file_name() == unknown.file_name())
    );
    assert!(first.backup_path.is_dir());

    let applied = BackupManager::apply_retention(&backups, policy, ABANDONED_STAGING_AGE, false)
        .expect("apply retention");
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

#[test]
fn backups_skip_benchmark_scaffolding_but_keep_runtime_configuration() {
    // The token-benchmark workflow stores run artefacts under runtime/token-benchmarks by
    // design (its isolation check requires run roots beneath BRAIN_HOME): frozen-brain copies
    // of ledgers that are themselves backed up, full git checkouts, and per-attempt harness
    // homes. Measured 2026-08-16, that tree was 22 GB of a 24 GB snapshot — copied hourly,
    // it filled a 1 TB drive in ten days. It is scaffolding, not evidence. What must survive
    // is the half a restore actually consumes: runtime/service.json (the service refuses to
    // start without it) alongside the ledgers.
    let fixture = fixture();
    let run = fixture
        .brain_home
        .join("runtime/token-benchmarks/019fcd85/run-1/frozen-brain");
    std::fs::create_dir_all(&run).expect("run directory");
    std::fs::write(run.join("frozen-tree.txt"), b"ledger copy").expect("frozen");
    std::fs::write(
        fixture.brain_home.join("runtime/service.json"),
        "{\"pipe_name\":\"test\"}\n",
    )
    .expect("service config");

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
        !captured
            .iter()
            .any(|path| path.starts_with("runtime/token-benchmarks/")),
        "benchmark scaffolding must not be captured, got {captured:?}"
    );
    assert!(
        captured.iter().any(|path| path == "runtime/service.json"),
        "runtime/service.json is restore-critical, got {captured:?}"
    );

    let restored =
        BackupManager::restore_isolated(&backup.backup_path, fixture.temp.path().join("restored"))
            .expect("restore");
    assert!(
        restored.destination.join("runtime/service.json").is_file(),
        "a restore must reinstate the service configuration"
    );
    assert!(
        !restored
            .destination
            .join("runtime/token-benchmarks")
            .exists(),
        "a restore reinstates the brain, not benchmark scaffolding"
    );
}

#[test]
fn apply_retention_sweeps_abandoned_staging_directories_by_age() {
    // A backup killed mid-copy (the task's PT2H execution limit does exactly this) leaves a
    // `.staging-<id>` directory behind: create() removes its own staging only when the
    // process is alive to see the error, and load_inventory classifies the orphan as
    // `ignored` forever. Measured 2026-08-16, ten of them held ~110 GB. Retention sweeps
    // them by age — never a young one, which may belong to a live run.
    let fixture = fixture();
    let backups = fixture.temp.path().join("backups");
    BackupManager::create(&fixture.brain_home, &backups, fixture.now).expect("backup");

    let abandoned = backups.join(format!(".staging-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&abandoned).expect("abandoned staging");
    std::fs::write(abandoned.join("partial.json"), b"{}").expect("partial file");

    let dry_run = BackupManager::apply_retention(
        &backups,
        RetentionPolicy::default(),
        std::time::Duration::ZERO,
        true,
    )
    .expect("dry run");
    assert!(
        dry_run
            .swept_staging
            .iter()
            .any(|path| path.file_name() == abandoned.file_name()),
        "an aged staging dir must be reported for sweeping, got {:?}",
        dry_run.swept_staging
    );
    assert!(abandoned.is_dir(), "dry run must not delete");

    let applied = BackupManager::apply_retention(
        &backups,
        RetentionPolicy::default(),
        std::time::Duration::ZERO,
        false,
    )
    .expect("apply");
    assert!(
        applied
            .swept_staging
            .iter()
            .any(|path| path.file_name() == abandoned.file_name())
    );
    assert!(
        !abandoned.exists(),
        "an aged staging dir is deleted on apply"
    );

    // A young staging dir may belong to a live run: it must survive every sweep and stay in
    // `ignored`, exactly as before the sweeper existed.
    let young = backups.join(format!(".staging-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&young).expect("young staging");
    let report = BackupManager::apply_retention(
        &backups,
        RetentionPolicy::default(),
        std::time::Duration::from_secs(6 * 60 * 60),
        false,
    )
    .expect("apply with age threshold");
    assert!(young.is_dir(), "a young staging dir is never swept");
    assert!(
        !report
            .swept_staging
            .iter()
            .any(|path| path.file_name() == young.file_name())
    );
    assert!(
        report
            .ignored
            .iter()
            .any(|path| path.file_name() == young.file_name()),
        "a young staging dir stays classified as ignored"
    );
}

#[test]
fn backups_verify_against_the_schema_the_live_population_actually_runs() {
    // The benchmark-era branch (b5e6e4e, agent/implement-second-brain-status-benchmark-*)
    // stamped an additive v10 onto the live ledgers while this branch carries byte-identical
    // DDL under the v9 stamp. A binary that refuses v10 therefore refuses to back up the
    // real brain — observed live on 2026-08-16, 213 seconds into a maintain that had
    // otherwise completed, failing only at the verify gate. The supported version tracks
    // the population; the schema is identical either way.
    let fixture = fixture();
    let connection = rusqlite::Connection::open(&fixture.ledger_path).expect("raw connection");
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (10, datetime('now'))",
            [],
        )
        .expect("stamp v10, as the benchmark-era binary did to the live brain");
    drop(connection);

    let backup = BackupManager::create(
        &fixture.brain_home,
        fixture.temp.path().join("backups"),
        fixture.now,
    )
    .expect("a backup of the live population's schema must verify");
    assert!(
        backup
            .inventory
            .files
            .iter()
            .any(|file| file.relative_path.ends_with("ledger.sqlite"))
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
