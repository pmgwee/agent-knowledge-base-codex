use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use brain_context::{ContextProvider, ContextQuery, ProviderResult, retrieve_provider_results};
use brain_domain::{ProjectId, WorktreeId};

#[tokio::test]
async fn slow_or_failed_providers_do_not_remove_fast_results_or_exceed_deadline() {
    let query = ContextQuery::for_worktree(
        ProjectId(uuid::Uuid::now_v7()),
        WorktreeId(uuid::Uuid::now_v7()),
    );
    let started = Instant::now();
    let results = retrieve_provider_results(
        vec![
            Arc::new(FastProvider),
            Arc::new(SlowProvider),
            Arc::new(FailedProvider),
        ],
        query,
        Duration::from_millis(25),
        Duration::from_millis(50),
    )
    .await;

    assert!(started.elapsed() < Duration::from_millis(200));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].provider, "fast");
}

struct FastProvider;
struct SlowProvider;
struct FailedProvider;

#[async_trait]
impl ContextProvider for FastProvider {
    async fn retrieve(&self, _query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        Ok(vec![ProviderResult {
            provider: "fast".to_owned(),
            title: "Useful result".to_owned(),
            content: "Canonical context remains and this optional result is added.".to_owned(),
            source_uri: "fixture://fast".to_owned(),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            trust: "external_document".to_owned(),
        }])
    }
}

#[async_trait]
impl ContextProvider for SlowProvider {
    async fn retrieve(&self, _query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(Vec::new())
    }
}

#[async_trait]
impl ContextProvider for FailedProvider {
    async fn retrieve(&self, _query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        anyhow::bail!("fixture outage")
    }
}
