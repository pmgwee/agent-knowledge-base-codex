use brain_context::{LlmWikiConfig, ProviderConfig, ProviderStatus};
use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, ProjectRegistry};
use brain_service::{
    BrainPromptContextRequest, BrainQueryService, HookProjectBinding, ProjectHookHandler,
    ServiceLaunchConfig, ServiceProjectConfig,
};
use brain_store::EventLedger;

#[test]
fn session_start_uses_cache_and_only_the_first_prompt_may_scan_the_wiki() {
    let temp = tempfile::tempdir().expect("create two-stage provider fixture");
    let brain_home = temp.path().join("brain");
    let project_root = temp.path().join("project");
    let wiki_vault = temp.path().join("llm-wiki-vault");
    std::fs::create_dir_all(&project_root).expect("create project root");
    std::fs::create_dir_all(&wiki_vault).expect("create wiki vault");
    std::fs::write(
        wiki_vault.join("authentication.md"),
        "# Authentication runbook\nUse PKCE for the OAuth callback and verify the state nonce.",
    )
    .expect("write reviewed wiki document");

    let identity = ProjectRegistry::open(&brain_home)
        .expect("open registry")
        .register(&project_root)
        .expect("register project");
    let project_storage = brain_home
        .join("projects")
        .join(identity.project_id.0.to_string());
    std::fs::create_dir_all(&project_storage).expect("create project storage");
    let ledger_path = project_storage.join("brain.sqlite");
    EventLedger::open(&ledger_path, identity.project_id).expect("initialize ledger");
    let providers = ProviderConfig {
        llm_wiki: LlmWikiConfig {
            enabled: true,
            vault: Some(wiki_vault.clone()),
            ..LlmWikiConfig::default()
        },
        ..ProviderConfig::default()
    };
    std::fs::write(
        project_storage.join("providers.json"),
        serde_json::to_vec_pretty(&providers).expect("serialize providers"),
    )
    .expect("write provider config");
    let service = BrainQueryService::from_config(
        &brain_home,
        ServiceLaunchConfig {
            projects: vec![ServiceProjectConfig {
                project_root: project_root.clone(),
                project_id: identity.project_id,
                worktree_id: identity.worktree_id,
                ledger_path: ledger_path.clone(),
                claude_sources: Vec::new(),
                codex_sources: Vec::new(),
                hermes_database: None,
            }],
            ..ServiceLaunchConfig::new("two-stage-test")
        },
    )
    .expect("open query service");

    let first = service
        .context_for_prompt(BrainPromptContextRequest {
            project: identity.project_id.0.to_string(),
            harness: Harness::ClaudeCode,
            native_session_id: "claude-session-1".to_owned(),
            prompt: "implement OAuth authentication callback".to_owned(),
            task_id: None,
            paths: vec!["src/auth.rs".to_owned()],
            max_tokens: Some(1_500),
        })
        .expect("retrieve first-prompt context");
    assert!(first.first_prompt_claimed);
    let wiki = first
        .providers
        .iter()
        .find(|state| state.provider == "llm_wiki")
        .expect("wiki provider state");
    assert_eq!(wiki.status, ProviderStatus::Ready);
    assert_eq!(wiki.item_count, 1);
    assert!(first.context.text.contains("Authentication runbook"));
    assert!(first.context.text.contains("authentication.md"));

    let handler = ProjectHookHandler::new(HookProjectBinding {
        project_root: project_root.clone(),
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        ledger_path,
        global_preferences_path: None,
    })
    .expect("open project hook handler");
    let startup = handler
        .handle(&HookEnvelope {
            protocol: HOOK_PROTOCOL_VERSION,
            nonce: uuid::Uuid::now_v7(),
            harness: Harness::Codex,
            event_name: "SessionStart".to_owned(),
            received_at: time::OffsetDateTime::now_utc(),
            payload: serde_json::json!({
                "cwd": project_root,
                "session_id": "codex-session-1"
            }),
        })
        .expect("compile cached startup context");
    let startup_context = startup.additional_context.expect("cached startup context");
    assert!(startup_context.contains("Authentication runbook"));
    assert!(startup_context.contains("external_document_cached"));

    let second = service
        .context_for_prompt(BrainPromptContextRequest {
            project: identity.project_id.0.to_string(),
            harness: Harness::ClaudeCode,
            native_session_id: "claude-session-1".to_owned(),
            prompt: "unrelated database migration".to_owned(),
            task_id: None,
            paths: Vec::new(),
            max_tokens: Some(1_500),
        })
        .expect("retrieve second-prompt context");
    assert!(!second.first_prompt_claimed);
    let wiki = second
        .providers
        .iter()
        .find(|state| state.provider == "llm_wiki")
        .expect("wiki provider state");
    assert_eq!(wiki.status, ProviderStatus::Disabled);
    assert_eq!(wiki.item_count, 0);
    assert!(
        wiki.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("already consumed"))
    );

    let retry = service
        .context_for_prompt(BrainPromptContextRequest {
            project: identity.project_id.0.to_string(),
            harness: Harness::ClaudeCode,
            native_session_id: "claude-session-1".to_owned(),
            prompt: "implement OAuth authentication callback".to_owned(),
            task_id: None,
            paths: vec!["src/auth.rs".to_owned()],
            max_tokens: Some(1_500),
        })
        .expect("retry cached first prompt");
    let wiki = retry
        .providers
        .iter()
        .find(|state| state.provider == "llm_wiki")
        .expect("wiki provider state");
    assert_eq!(wiki.status, ProviderStatus::Ready);
    assert_eq!(wiki.item_count, 1);
    assert!(
        wiki.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("idempotent"))
    );
}
