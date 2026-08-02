use brain_domain::{
    EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, UpgradeManager};
use sha2::{Digest, Sha256};

#[test]
fn staged_upgrade_preserves_raw_event_hashes_and_never_replaces_active_state() {
    let fixture = fixture();
    let source_before = UpgradeManager::check(&fixture.brain_home).expect("source check");
    let staged_path = fixture.temp.path().join("staged brain");
    let report = UpgradeManager::stage(&fixture.brain_home, &staged_path, fixture.now)
        .expect("stage upgrade");
    assert!(report.raw_hashes_preserved);
    assert_eq!(report.source.raw_event_hash, report.staged.raw_event_hash);
    assert_eq!(source_before.raw_event_hash, report.source.raw_event_hash);
    assert!(fixture.brain_home.join("sentinel.txt").is_file());
    assert!(staged_path.join("sentinel.txt").is_file());
}

#[test]
fn newer_unknown_format_is_refused_read_only() {
    let fixture = fixture();
    std::fs::write(
        fixture.brain_home.join("projects.json"),
        "{\"schema_version\":999,\"projects\":[]}",
    )
    .expect("newer registry");
    let report = UpgradeManager::check(&fixture.brain_home).expect("check");
    assert!(!report.compatible);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.message.contains("999"))
    );
    assert!(fixture.brain_home.join("sentinel.txt").is_file());
}

struct Fixture {
    temp: tempfile::TempDir,
    brain_home: std::path::PathBuf,
    now: time::OffsetDateTime,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let project = ProjectId(uuid::Uuid::now_v7());
    let worktree = WorktreeId(uuid::Uuid::now_v7());
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let mut ledger =
        EventLedger::open(brain_home.join("projects/p1/ledger.sqlite"), project).expect("ledger");
    let raw = serde_json::json!({"fact": "preserve me"});
    ledger
        .append_batch(&EventBatch {
            source_id: "fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id: uuid::Uuid::now_v7(),
                project_id: project,
                worktree_id: worktree,
                task_id: None,
                harness: Harness::Codex,
                native_session_id: "upgrade".to_owned(),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at: now,
                observed_at: now,
                source_locator: "fixture".to_owned(),
                source_offset: 0,
                source_schema: "fixture:v1".to_owned(),
                raw_hash: Sha256::digest(serde_json::to_vec(&raw).expect("raw")).into(),
                idempotency_key: Sha256::digest(b"upgrade-fixture").into(),
                git_head: None,
                git_branch: None,
                payload: raw.clone(),
                raw,
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("append");
    std::fs::write(brain_home.join("sentinel.txt"), "active\n").expect("sentinel");
    std::fs::write(
        brain_home.join("projects.json"),
        "{\"schema_version\":1,\"projects\":[]}",
    )
    .expect("registry");
    Fixture {
        temp,
        brain_home,
        now,
    }
}
