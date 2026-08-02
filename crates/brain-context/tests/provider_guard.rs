use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use brain_context::{ContextProvider, ContextQuery, ProviderGuard, ProviderResult, ProviderStatus};
use brain_domain::{ProjectId, WorktreeId};

#[tokio::test]
async fn disabled_provider_is_never_invoked() {
    let provider = Arc::new(CountingProvider::new(Mode::Valid));
    let guard = ProviderGuard::new(
        "fixture",
        provider.clone(),
        false,
        Duration::from_millis(20),
        Duration::from_secs(60),
    );
    let result = guard.retrieve(&query()).await;
    assert_eq!(result.status, ProviderStatus::Disabled);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn timeout_scope_violation_and_circuit_fail_open_without_foreign_results() {
    let hanging = Arc::new(CountingProvider::new(Mode::Hang));
    let guard = ProviderGuard::new(
        "hanging",
        hanging,
        true,
        Duration::from_millis(10),
        Duration::from_secs(60),
    );
    let result = guard.retrieve(&query()).await;
    assert_eq!(result.status, ProviderStatus::TimedOut);
    assert!(result.items.is_empty());

    let foreign = Arc::new(CountingProvider::new(Mode::Foreign));
    let guard = ProviderGuard::new(
        "foreign",
        foreign,
        true,
        Duration::from_millis(50),
        Duration::from_secs(60),
    );
    let result = guard.retrieve(&query()).await;
    assert_eq!(result.status, ProviderStatus::ScopeViolation);
    assert!(result.items.is_empty());
}

enum Mode {
    Valid,
    Hang,
    Foreign,
}

struct CountingProvider {
    calls: AtomicUsize,
    mode: Mode,
}

impl CountingProvider {
    fn new(mode: Mode) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            mode,
        }
    }
}

#[async_trait]
impl ContextProvider for CountingProvider {
    async fn retrieve(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::Hang) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let project_id = if matches!(self.mode, Mode::Foreign) {
            ProjectId(uuid::Uuid::now_v7())
        } else {
            query.project_id
        };
        Ok(vec![ProviderResult {
            provider: "fixture".to_owned(),
            project_id,
            worktree_id: Some(query.worktree_id),
            title: "Result".to_owned(),
            content: "Scoped result".to_owned(),
            source_uri: "fixture://result".to_owned(),
            source_date: Some(time::OffsetDateTime::UNIX_EPOCH),
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            trust: "external_document".to_owned(),
            relevance: 1.0,
            git_head: None,
        }])
    }
}

fn query() -> ContextQuery {
    ContextQuery::for_worktree(
        ProjectId(uuid::Uuid::now_v7()),
        WorktreeId(uuid::Uuid::now_v7()),
    )
}
