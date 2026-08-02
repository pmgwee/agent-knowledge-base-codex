use brain_cli::{
    RegisterOptions, configure_llm_wiki, provider_status, register_project, remove_provider,
};
use brain_domain::{EventBatch, EventType, Harness, NormalizedEvent, SourceCursor};
use brain_service::{BrainPromptContextRequest, BrainQueryService};
use brain_store::EventLedger;

#[test]
fn removing_optional_provider_preserves_canonical_history_and_external_vault() {
    let temp = tempfile::tempdir().expect("create provider lifecycle fixture");
    let brain_home = temp.path().join("brain");
    let project_root = temp.path().join("project");
    let wiki_vault = temp.path().join("reviewed-wiki");
    std::fs::create_dir_all(&project_root).expect("create project");
    std::fs::create_dir_all(&wiki_vault).expect("create wiki");
    let wiki_document = wiki_vault.join("oauth.md");
    std::fs::write(
        &wiki_document,
        "# OAuth guide\nValidate the callback state nonce.",
    )
    .expect("write wiki document");
    let registered = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project_root,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: None,
    })
    .expect("register project");
    let event_id = uuid::Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    let mut ledger = EventLedger::open(&registered.ledger_path, registered.project_id)
        .expect("open canonical ledger");
    ledger
        .append_batch(&EventBatch {
            source_id: "provider-lifecycle-fixture".to_owned(),
            events: vec![NormalizedEvent {
                event_id,
                project_id: registered.project_id,
                worktree_id: registered.worktree_id,
                task_id: None,
                harness: Harness::Codex,
                native_session_id: "session-1".to_owned(),
                native_turn_id: Some("turn-1".to_owned()),
                event_type: EventType::UserPrompted,
                occurred_at: now,
                observed_at: now,
                source_locator: "fixture.jsonl".to_owned(),
                source_offset: 1,
                source_schema: "fixture-v1".to_owned(),
                raw_hash: [1; 32],
                idempotency_key: [2; 32],
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({"text": "retain canonical OAuth evidence"}),
                raw: serde_json::json!({"text": "retain canonical OAuth evidence"}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })
        .expect("seed canonical history");
    let canonical_before = ledger
        .recent_events(registered.project_id, 50)
        .expect("read canonical events");
    drop(ledger);

    configure_llm_wiki(
        &brain_home,
        &registered.project_id.0.to_string(),
        &wiki_vault,
    )
    .expect("enable wiki");
    BrainQueryService::open(&brain_home)
        .expect("open service")
        .context_for_prompt(BrainPromptContextRequest {
            project: registered.project_id.0.to_string(),
            harness: Harness::Codex,
            native_session_id: "session-1".to_owned(),
            prompt: "OAuth callback state".to_owned(),
            task_id: None,
            paths: Vec::new(),
            max_tokens: None,
        })
        .expect("populate provider cache");
    let removed = remove_provider(
        &brain_home,
        &registered.project_id.0.to_string(),
        brain_cli::ProviderKind::LlmWiki,
    )
    .expect("remove wiki provider");
    assert_eq!(removed.removed_cache_entries, 1);
    assert!(removed.external_data_preserved);
    assert!(wiki_document.is_file());

    let status = provider_status(&brain_home, &registered.project_id.0.to_string())
        .expect("read provider status");
    let wiki = status
        .iter()
        .find(|status| status.provider == brain_cli::ProviderKind::LlmWiki)
        .expect("wiki status");
    assert!(!wiki.enabled);
    assert!(!wiki.usable);
    let ledger = EventLedger::open(&registered.ledger_path, registered.project_id)
        .expect("reopen canonical ledger");
    let canonical_after = ledger
        .recent_events(registered.project_id, 50)
        .expect("read canonical events after removal");
    assert_eq!(canonical_after, canonical_before);
    assert_eq!(ledger.event_count().expect("count events"), 1);
    assert!(ledger.event(event_id).expect("read event").is_some());
}
