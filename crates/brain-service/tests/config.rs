use std::time::Duration;

use brain_service::{CaptureServiceConfig, ConsolidationProviderConfig, ServiceLaunchConfig};

#[test]
fn capture_timing_rejects_zero_intervals() {
    let invalid_reconciliation = CaptureServiceConfig {
        reconciliation_interval: Duration::ZERO,
        ..CaptureServiceConfig::default()
    };
    assert!(invalid_reconciliation.validate().is_err());

    let invalid_debounce = CaptureServiceConfig {
        watcher_debounce: Duration::ZERO,
        ..CaptureServiceConfig::default()
    };
    assert!(invalid_debounce.validate().is_err());
}

#[test]
fn capture_timing_defaults_match_the_recovery_contract() {
    let config = CaptureServiceConfig::default();
    assert_eq!(config.reconciliation_interval, Duration::from_secs(2));
    assert_eq!(config.watcher_debounce, Duration::from_millis(50));
    config.validate().expect("default timing is valid");
}

#[test]
fn service_launch_config_loads_the_explicit_project_scope() {
    let temp = tempfile::tempdir().expect("create service config fixture");
    let project_id = uuid::Uuid::now_v7();
    let worktree_id = uuid::Uuid::now_v7();
    let path = temp.path().join("service.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "pipe_name": r"\\.\pipe\fixture-brain",
            "project_root": temp.path().join("project"),
            "project_id": project_id,
            "worktree_id": worktree_id,
            "ledger_path": temp.path().join("events.db"),
            "claude_sources": [temp.path().join("session.jsonl")],
        }))
        .expect("serialize fixture config"),
    )
    .expect("write service config");

    let config = ServiceLaunchConfig::load(&path).expect("load service config");

    let project = config.project(None).expect("select migrated project");
    assert_eq!(project.project_id.0, project_id);
    assert_eq!(project.worktree_id.0, worktree_id);
    assert_eq!(project.claude_sources.len(), 1);
    assert_eq!(config.schema_version, 2);
}

#[test]
fn glm_consolidation_defaults_are_explicit_and_opt_in() {
    let temp = tempfile::tempdir().expect("create service config fixture");
    let path = temp.path().join("service.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "pipe_name": r"\\.\pipe\fixture-brain",
            "consolidation": {
                "provider": "glm",
                "endpoint": "https://example.invalid/v1/chat/completions",
                "model": "glm-fixture"
            },
            "projects": []
        }))
        .expect("serialize fixture config"),
    )
    .expect("write service config");

    let config = ServiceLaunchConfig::load(&path).expect("load service config");
    match config.consolidation.expect("GLM is configured") {
        ConsolidationProviderConfig::Glm {
            api_key_env,
            timeout_ms,
            max_retries,
            ..
        } => {
            assert_eq!(api_key_env, "GLM_API_KEY");
            assert_eq!(timeout_ms, 30_000);
            assert_eq!(max_retries, 2);
        }
    }
}
