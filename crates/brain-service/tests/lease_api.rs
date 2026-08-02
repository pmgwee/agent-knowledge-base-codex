use std::fs;

use brain_coordination::{CoordinationStore, SessionIdentity, TaskRecord, TaskStatus};
use brain_domain::{Harness, ProjectRegistry};
use brain_service::{
    BrainLeaseAcquireRequest, BrainLeaseHandoffRequest, BrainQueryService, ServiceLaunchConfig,
    ServiceProjectConfig,
};
use brain_store::EventLedger;

#[test]
fn handoff_writes_a_checkpoint_transfers_generation_and_replays_idempotently() {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("project");
    let identity = ProjectRegistry::open(&brain_home)
        .expect("registry")
        .register(&project_root)
        .expect("register");
    let ledger_path = temp.path().join("ledger.sqlite");
    EventLedger::open(&ledger_path, identity.project_id).expect("ledger");
    let now = time::OffsetDateTime::now_utc();
    let task = TaskRecord {
        id: uuid::Uuid::now_v7(),
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        title: "OAuth".to_owned(),
        worktree_path: Some(identity.root.clone()),
        branch: None,
        status: TaskStatus::Active,
        created_at: now,
        closed_at: None,
    };
    CoordinationStore::open(&ledger_path, identity.project_id)
        .expect("coordination")
        .create_task(&task)
        .expect("task");
    let mut config = ServiceLaunchConfig::new(r"\\.\pipe\lease-api");
    config.upsert_project(ServiceProjectConfig {
        project_root: identity.root,
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        ledger_path: ledger_path.clone(),
        claude_sources: Vec::new(),
        codex_sources: Vec::new(),
        hermes_database: None,
    });
    let service = BrainQueryService::from_config(&brain_home, config).expect("service");
    let claude = owner(Harness::ClaudeCode, "claude");
    let codex = owner(Harness::Codex, "codex");
    let lease = service
        .acquire_lease(BrainLeaseAcquireRequest {
            project: identity.project_id.0.to_string(),
            task_id: task.id,
            owner: claude.clone(),
        })
        .expect("acquire")
        .lease
        .expect("lease");
    let handoff_id = uuid::Uuid::now_v7();
    let request = BrainLeaseHandoffRequest {
        project: identity.project_id.0.to_string(),
        handoff_id,
        task_id: task.id,
        current_owner: claude,
        generation: lease.generation,
        next_owner: codex.clone(),
        checkpoint: "OAuth callback fixed; next add PKCE".to_owned(),
    };
    let handed = service.handoff_lease(request.clone()).expect("handoff");
    assert_eq!(handed.lease.as_ref().expect("handed lease").owner, codex);
    assert_eq!(
        handed.lease.as_ref().expect("handed lease").generation,
        lease.generation + 1
    );
    let replay = service.handoff_lease(request).expect("replay");
    assert!(replay.replayed);
    let event = EventLedger::open(&ledger_path, identity.project_id)
        .expect("ledger")
        .event(handoff_id)
        .expect("event query")
        .expect("handoff checkpoint event");
    assert_eq!(
        event.event_type,
        brain_domain::EventType::CheckpointAuthored
    );
}

fn owner(harness: Harness, session: &str) -> SessionIdentity {
    SessionIdentity {
        harness,
        native_session_id: session.to_owned(),
    }
}
