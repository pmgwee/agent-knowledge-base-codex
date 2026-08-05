use std::fs;

use brain_domain::{Harness, ProjectRegistry};
use brain_service::{
    BrainCheckpointRequest, BrainQueryService, ServiceLaunchConfig, ServiceProjectConfig,
};
use brain_store::EventLedger;

fn service_with_project() -> (
    tempfile::TempDir,
    BrainQueryService,
    brain_domain::ProjectId,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("project");

    let identity = ProjectRegistry::open(&brain_home)
        .expect("registry")
        .register(&project_root)
        .expect("register");

    let ledger_path = brain_home
        .join("projects")
        .join(identity.project_id.0.to_string())
        .join("evidence")
        .join("hot")
        .join("events.sqlite");
    EventLedger::open(&ledger_path, identity.project_id).expect("ledger");

    let config = ServiceLaunchConfig {
        schema_version: 2,
        pipe_name: "test-pipe".to_owned(),
        consolidation: None,
        projects: vec![ServiceProjectConfig {
            project_root: identity.root.clone(),
            project_id: identity.project_id,
            worktree_id: identity.worktree_id,
            ledger_path: ledger_path.clone(),
            claude_sources: Vec::new(),
            codex_sources: Vec::new(),
            hermes_database: None,
        }],
    };
    let service = BrainQueryService::from_config(&brain_home, config).expect("service");
    (temp, service, identity.project_id, ledger_path)
}

#[test]
fn checkpoint_via_mcp_records_delivery_with_harness_attribution() {
    let (_temp, service, project_id, ledger_path) = service_with_project();

    service
        .checkpoint(BrainCheckpointRequest {
            project: project_id.0.to_string(),
            prompt: Some("what was I working on".to_owned()),
            paths: Vec::new(),
            as_of: None,
            max_tokens: None,
            harness: Some(Harness::Codex),
            native_session_id: Some("codex-session-42".to_owned()),
        })
        .expect("checkpoint");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    let summary = ledger
        .context_delivery_summary(time::OffsetDateTime::UNIX_EPOCH)
        .expect("summary");

    assert_eq!(summary.deliveries, 1, "the MCP checkpoint must be recorded");
    assert!(summary.max_tokens > 0, "token count must be non-zero");
    assert!(
        summary.max_tokens <= 1500,
        "must respect the token budget, got {}",
        summary.max_tokens
    );
}

#[test]
fn checkpoint_without_harness_still_records() {
    let (_temp, service, project_id, ledger_path) = service_with_project();

    service
        .checkpoint(BrainCheckpointRequest {
            project: project_id.0.to_string(),
            prompt: None,
            paths: Vec::new(),
            as_of: None,
            max_tokens: None,
            harness: None,
            native_session_id: None,
        })
        .expect("checkpoint");

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    let summary = ledger
        .context_delivery_summary(time::OffsetDateTime::UNIX_EPOCH)
        .expect("summary");

    assert_eq!(summary.deliveries, 1);
}

#[test]
fn multiple_checkpoints_accumulate_in_the_delivery_log() {
    let (_temp, service, project_id, ledger_path) = service_with_project();

    for i in 0..3 {
        service
            .checkpoint(BrainCheckpointRequest {
                project: project_id.0.to_string(),
                prompt: Some(format!("task {i}")),
                paths: Vec::new(),
                as_of: None,
                max_tokens: None,
                harness: Some(Harness::Codex),
                native_session_id: Some(format!("session-{i}")),
            })
            .expect("checkpoint");
    }

    let ledger = EventLedger::open(&ledger_path, project_id).expect("reopen");
    let summary = ledger
        .context_delivery_summary(time::OffsetDateTime::UNIX_EPOCH)
        .expect("summary");

    assert_eq!(summary.deliveries, 3, "each MCP call must be recorded");
    assert!(summary.total_tokens > 0);
}

#[test]
fn checkpoint_prepends_coordination_context_so_codex_sees_active_state() {
    // Codex's desktop app does not fire the SessionStart hook, so brain_checkpoint is the only
    // channel through which it learns about leases and path claims. The checkpoint must prepend
    // the same coordination view the hook injects for Claude Code.
    let (_temp, service, project_id, _ledger_path) = service_with_project();

    let response = service
        .checkpoint(BrainCheckpointRequest {
            project: project_id.0.to_string(),
            prompt: None,
            paths: Vec::new(),
            as_of: None,
            max_tokens: None,
            harness: Some(Harness::Codex),
            native_session_id: Some("codex-coord-test".to_owned()),
        })
        .expect("checkpoint");

    assert!(
        response.context.text.contains("Coordination state"),
        "brain_checkpoint must prepend coordination context for Codex; got: {}",
        response.context.text
    );
    assert!(
        response.context.token_count <= 1_500,
        "coordination + memory must stay within the token budget, got {}",
        response.context.token_count
    );
}
