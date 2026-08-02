use brain_domain::{Harness, ProjectId};
use brain_store::{ProviderCacheEntry, ProviderCacheStore};

#[test]
fn cache_expiry_config_invalidation_scope_and_first_prompt_are_enforced() {
    let temp = tempfile::tempdir().expect("temp");
    let project = ProjectId(uuid::Uuid::now_v7());
    let other = ProjectId(uuid::Uuid::now_v7());
    let path = temp.path().join("ledger.sqlite");
    brain_store::EventLedger::open(&path, project).expect("ledger");
    let mut store = ProviderCacheStore::open(&path, project).expect("cache");
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000);
    let query = [1_u8; 32];
    let config = [2_u8; 32];
    store
        .put(&ProviderCacheEntry {
            provider: "llm_wiki".to_owned(),
            project_id: project,
            task_id: None,
            query_sha256: query,
            config_sha256: config,
            source_version: "v1".to_owned(),
            fetched_at: now,
            expires_at: now + time::Duration::hours(1),
            items: serde_json::json!([{"title": "cached"}]),
        })
        .expect("put");
    assert!(
        store
            .get("llm_wiki", query, config, "v1", now)
            .expect("get")
            .is_some()
    );
    assert!(
        store
            .get("llm_wiki", query, [3; 32], "v1", now)
            .expect("wrong config")
            .is_none()
    );
    assert!(
        store
            .get(
                "llm_wiki",
                query,
                config,
                "v1",
                now + time::Duration::hours(2)
            )
            .expect("expired")
            .is_none()
    );
    assert!(
        store
            .put(&ProviderCacheEntry {
                provider: "llm_wiki".to_owned(),
                project_id: other,
                task_id: None,
                query_sha256: query,
                config_sha256: config,
                source_version: "v1".to_owned(),
                fetched_at: now,
                expires_at: now + time::Duration::hours(1),
                items: serde_json::json!([]),
            })
            .is_err()
    );
    assert!(
        store
            .claim_first_prompt(&Harness::Codex, "session", query, now)
            .expect("claim")
    );
    assert!(
        !store
            .claim_first_prompt(&Harness::Codex, "session", [9; 32], now)
            .expect("retry")
    );
}
