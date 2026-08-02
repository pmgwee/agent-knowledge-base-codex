use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::ContextQuery;

#[derive(Clone, Debug)]
pub struct ProviderResult {
    pub provider: String,
    pub title: String,
    pub content: String,
    pub source_uri: String,
    pub observed_at: time::OffsetDateTime,
    pub trust: String,
}

#[async_trait]
pub trait ContextProvider: Send + Sync {
    async fn retrieve(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>>;
}

pub async fn retrieve_provider_results(
    providers: Vec<Arc<dyn ContextProvider>>,
    query: ContextQuery,
    per_provider_timeout: Duration,
    total_timeout: Duration,
) -> Vec<ProviderResult> {
    let mut tasks = tokio::task::JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            tokio::time::timeout(per_provider_timeout, provider.retrieve(&query)).await
        });
    }
    let deadline = tokio::time::Instant::now() + total_timeout;
    let mut results = Vec::new();
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            tasks.abort_all();
            break;
        };
        match tokio::time::timeout(remaining, tasks.join_next()).await {
            Ok(Some(Ok(Ok(Ok(items))))) => {
                results.extend(items.into_iter().take(8).filter_map(validate_result));
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                tasks.abort_all();
                break;
            }
        }
    }
    results.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.source_uri.cmp(&right.source_uri))
    });
    results.truncate(32);
    results
}

fn validate_result(mut result: ProviderResult) -> Option<ProviderResult> {
    if result.provider.trim().is_empty()
        || result.title.trim().is_empty()
        || result.content.trim().is_empty()
        || result.source_uri.trim().is_empty()
    {
        return None;
    }
    result.provider = compact(&result.provider, 80);
    result.title = compact(&result.title, 300);
    result.content = compact(&result.content, 4_000);
    result.source_uri = compact(&result.source_uri, 500);
    result.trust = compact(&result.trust, 80);
    Some(result)
}

fn compact(value: &str, max: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}
