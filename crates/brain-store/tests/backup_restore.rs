use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{BackupManager, EventLedger};
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
