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
fn llm_consolidation_defaults_to_the_configured_provider_and_stays_opt_in() {
    // The block may name nothing but the provider: base URL, model, key variable, timeout and
    // retries all have defaults, so a deployment that uses the standard endpoint configures
    // nothing here and sets one secret in its environment.
    let temp = tempfile::tempdir().expect("create service config fixture");
    let path = temp.path().join("service.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "pipe_name": r"\\.\pipe\fixture-brain",
            "consolidation": {"provider": "llm"},
            "projects": []
        }))
        .expect("serialize fixture config"),
    )
    .expect("write service config");

    let config = ServiceLaunchConfig::load(&path).expect("load service config");
    let ConsolidationProviderConfig::Llm {
        base_url,
        model,
        api_key_env,
        timeout_ms,
        max_retries,
    } = config.consolidation.expect("a provider is configured");
    assert_eq!(base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(model, "gpt-5.6-luna");
    assert_eq!(api_key_env, "LLM_API_KEY");
    assert_eq!(timeout_ms, 30_000);
    assert_eq!(max_retries, 2);
}

#[test]
fn consolidation_is_still_opt_in() {
    let temp = tempfile::tempdir().expect("create service config fixture");
    let path = temp.path().join("service.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "pipe_name": r"\\.\pipe\fixture-brain",
            "projects": []
        }))
        .expect("serialize fixture config"),
    )
    .expect("write service config");

    let config = ServiceLaunchConfig::load(&path).expect("load service config");
    assert!(
        config.consolidation.is_none(),
        "a configuration that says nothing about consolidation does not get a provider"
    );
}

#[test]
fn a_consolidation_block_this_build_cannot_read_disables_consolidation_without_stopping_anything() {
    // What a configuration file left over from the previous provider looks like. Failing the
    // whole load would take the service, every CLI command and the dashboard down with it —
    // for an optional feature. Consolidation is the only thing that stops.
    let temp = tempfile::tempdir().expect("create service config fixture");
    let path = temp.path().join("service.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "pipe_name": r"\\.\pipe\fixture-brain",
            "consolidation": {
                "provider": "a_provider_this_build_has_never_heard_of",
                "endpoint": "https://example.invalid/v1/chat/completions",
                "model": "something-retired"
            },
            "projects": []
        }))
        .expect("serialize fixture config"),
    )
    .expect("write service config");

    let config = ServiceLaunchConfig::load(&path)
        .expect("an unreadable consolidation block must not fail the load");
    assert!(
        config.consolidation.is_none(),
        "an unreadable block must not be interpreted as a provider"
    );
}

#[test]
fn resolving_a_provider_validates_it_and_names_what_is_missing() {
    let sound = ConsolidationProviderConfig::Llm {
        base_url: "https://opencode.ai/zen/go/v1".to_owned(),
        model: "gpt-5.6-luna".to_owned(),
        api_key_env: "LLM_API_KEY".to_owned(),
        timeout_ms: 30_000,
        max_retries: 2,
    };
    let resolved = sound.resolve().expect("a sound provider resolves");
    assert_eq!(resolved.base_url, "https://opencode.ai/zen/go/v1");
    assert_eq!(resolved.model, "gpt-5.6-luna");
    assert_eq!(resolved.api_key_env, "LLM_API_KEY");
    assert_eq!(resolved.timeout, std::time::Duration::from_millis(30_000));
    sound
        .client()
        .expect("a sound provider builds a client without reading the key");

    let ConsolidationProviderConfig::Llm { base_url, .. } = &sound;
    let no_model = ConsolidationProviderConfig::Llm {
        base_url: base_url.clone(),
        model: "   ".to_owned(),
        api_key_env: "LLM_API_KEY".to_owned(),
        timeout_ms: 30_000,
        max_retries: 2,
    };
    let error = no_model.resolve().expect_err("an empty model is a failure");
    let text = format!("{error:#}");
    assert!(
        text.contains("LLM_MODEL"),
        "the error must name the setting to supply, got: {text}"
    );
}
