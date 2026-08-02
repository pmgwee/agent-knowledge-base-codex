use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use brain_domain::{ProjectId, WorktreeId};

use crate::ContextQuery;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProviderResult {
    pub provider: String,
    pub project_id: ProjectId,
    #[serde(default)]
    pub worktree_id: Option<WorktreeId>,
    pub title: String,
    pub content: String,
    pub source_uri: String,
    #[serde(default)]
    pub source_date: Option<time::OffsetDateTime>,
    pub observed_at: time::OffsetDateTime,
    pub trust: String,
    #[serde(default = "default_relevance")]
    pub relevance: f64,
    #[serde(default)]
    pub git_head: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Disabled,
    Ready,
    TimedOut,
    Failed,
    ScopeViolation,
    Malformed,
    CircuitOpen,
    Stale,
    Unsupported,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct GuardedProviderResult {
    pub provider: String,
    pub status: ProviderStatus,
    pub items: Vec<ProviderResult>,
    pub latency_ms: u64,
    pub last_success: Option<time::OffsetDateTime>,
    pub cache_age_seconds: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub codegraph: CodeGraphConfig,
    #[serde(default)]
    pub llm_wiki: LlmWikiConfig,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub executable: Option<PathBuf>,
    #[serde(default = "enabled_by_default")]
    pub per_worktree_indexes: bool,
    #[serde(default = "default_codegraph_deadline_ms")]
    pub deadline_ms: u64,
    #[serde(default)]
    pub activation_report_sha256: Option<[u8; 32]>,
}

impl Default for CodeGraphConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable: None,
            per_worktree_indexes: true,
            deadline_ms: default_codegraph_deadline_ms(),
            activation_report_sha256: None,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct LlmWikiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub vault: Option<PathBuf>,
    #[serde(default = "default_llm_wiki_deadline_ms")]
    pub deadline_ms: u64,
    #[serde(default = "default_llm_wiki_results")]
    pub max_results: usize,
    #[serde(default = "default_llm_wiki_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: u64,
}

impl Default for LlmWikiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            vault: None,
            deadline_ms: default_llm_wiki_deadline_ms(),
            max_results: default_llm_wiki_results(),
            max_tokens: default_llm_wiki_tokens(),
            cache_ttl_seconds: default_cache_ttl_seconds(),
        }
    }
}

impl ProviderConfig {
    pub fn load(path: impl AsRef<Path>, brain_home: &Path) -> Result<Self> {
        let config = if path.as_ref().is_file() {
            serde_json::from_slice(&std::fs::read(path.as_ref())?)?
        } else {
            Self::default()
        };
        config.validate(brain_home)?;
        Ok(config)
    }

    pub fn sha256(&self) -> Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        Ok(Sha256::digest(serde_json::to_vec(self)?).into())
    }

    pub fn validate(&self, brain_home: &Path) -> Result<()> {
        ensure!(
            self.codegraph.deadline_ms > 0 && self.codegraph.deadline_ms <= 30_000,
            "CodeGraph deadline must be between 1 and 30000 ms"
        );
        ensure!(
            self.codegraph.per_worktree_indexes,
            "CodeGraph must use separate indexes per worktree"
        );
        ensure!(
            self.llm_wiki.deadline_ms > 0 && self.llm_wiki.deadline_ms <= 300,
            "LLM Wiki hook deadline must be between 1 and 300 ms"
        );
        ensure!(
            self.llm_wiki.max_results > 0 && self.llm_wiki.max_results <= 3,
            "LLM Wiki may contribute at most three results"
        );
        ensure!(
            self.llm_wiki.max_tokens > 0 && self.llm_wiki.max_tokens <= 600,
            "LLM Wiki may contribute at most 600 tokens"
        );
        ensure!(
            self.llm_wiki.cache_ttl_seconds > 0,
            "provider cache TTL is zero"
        );
        if let Some(vault) = &self.llm_wiki.vault {
            let home = normalized(brain_home);
            let vault = normalized(vault);
            ensure!(
                !is_same_or_child(&vault, &home) && !is_same_or_child(&home, &vault),
                "LLM Wiki vault must be separate from the canonical brain"
            );
        }
        Ok(())
    }
}

#[async_trait]
pub trait ContextProvider: Send + Sync {
    async fn retrieve(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>>;
}

struct CircuitState {
    consecutive_failures: u8,
    open_until: Option<time::OffsetDateTime>,
    last_success: Option<time::OffsetDateTime>,
    cache: Vec<ProviderResult>,
    cache_time: Option<time::OffsetDateTime>,
}

pub struct ProviderGuard {
    name: String,
    provider: Arc<dyn ContextProvider>,
    enabled: bool,
    deadline: Duration,
    cache_ttl: Duration,
    state: Mutex<CircuitState>,
}

impl ProviderGuard {
    pub fn new(
        name: impl Into<String>,
        provider: Arc<dyn ContextProvider>,
        enabled: bool,
        deadline: Duration,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            name: name.into(),
            provider,
            enabled,
            deadline,
            cache_ttl,
            state: Mutex::new(CircuitState {
                consecutive_failures: 0,
                open_until: None,
                last_success: None,
                cache: Vec::new(),
                cache_time: None,
            }),
        }
    }

    pub async fn retrieve(&self, query: &ContextQuery) -> GuardedProviderResult {
        let started = Instant::now();
        if !self.enabled {
            return self.response(ProviderStatus::Disabled, Vec::new(), started, None);
        }
        let now = time::OffsetDateTime::now_utc();
        {
            let state = self.state.lock().expect("provider circuit lock");
            if state.open_until.is_some_and(|until| now < until) {
                let cache = valid_cache(&state, query.project_id, now, self.cache_ttl);
                drop(state);
                return self.response(
                    ProviderStatus::CircuitOpen,
                    cache,
                    started,
                    Some("circuit is open after three provider failures".to_owned()),
                );
            }
        }
        match tokio::time::timeout(self.deadline, self.provider.retrieve(query)).await {
            Err(_) => self.failure(
                ProviderStatus::TimedOut,
                query.project_id,
                now,
                started,
                "provider deadline exceeded",
            ),
            Ok(Err(error)) => self.failure(
                ProviderStatus::Failed,
                query.project_id,
                now,
                started,
                &error.to_string(),
            ),
            Ok(Ok(items)) => {
                let mut valid = Vec::new();
                let mut malformed = false;
                let mut scope_violation = false;
                for item in items {
                    if item.project_id != query.project_id
                        || item.worktree_id.is_some_and(|id| id != query.worktree_id)
                    {
                        scope_violation = true;
                        continue;
                    }
                    if let Some(item) = validate_result(item) {
                        valid.push(item);
                    } else {
                        malformed = true;
                    }
                }
                if scope_violation {
                    return self.failure(
                        ProviderStatus::ScopeViolation,
                        query.project_id,
                        now,
                        started,
                        "provider returned another project or worktree",
                    );
                }
                if malformed && valid.is_empty() {
                    return self.failure(
                        ProviderStatus::Malformed,
                        query.project_id,
                        now,
                        started,
                        "provider returned malformed or uncited results",
                    );
                }
                let mut state = self.state.lock().expect("provider circuit lock");
                state.consecutive_failures = 0;
                state.open_until = None;
                state.last_success = Some(now);
                state.cache = valid.clone();
                state.cache_time = Some(now);
                drop(state);
                self.response(ProviderStatus::Ready, valid, started, None)
            }
        }
    }

    fn failure(
        &self,
        status: ProviderStatus,
        project_id: ProjectId,
        now: time::OffsetDateTime,
        started: Instant,
        reason: &str,
    ) -> GuardedProviderResult {
        let mut state = self.state.lock().expect("provider circuit lock");
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.consecutive_failures >= 3 {
            state.open_until = Some(now + time::Duration::minutes(5));
        }
        let cache = valid_cache(&state, project_id, now, self.cache_ttl);
        drop(state);
        self.response(status, cache, started, Some(compact(reason, 300)))
    }

    fn response(
        &self,
        status: ProviderStatus,
        items: Vec<ProviderResult>,
        started: Instant,
        reason: Option<String>,
    ) -> GuardedProviderResult {
        let state = self.state.lock().expect("provider circuit lock");
        GuardedProviderResult {
            provider: self.name.clone(),
            status,
            items,
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            last_success: state.last_success,
            cache_age_seconds: state.cache_time.and_then(|cached| {
                u64::try_from((time::OffsetDateTime::now_utc() - cached).whole_seconds()).ok()
            }),
            reason,
        }
    }
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
            let project = query.project_id;
            let worktree = query.worktree_id;
            (
                project,
                worktree,
                tokio::time::timeout(per_provider_timeout, provider.retrieve(&query)).await,
            )
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
            Ok(Some(Ok((project, worktree, Ok(Ok(items)))))) => {
                results.extend(
                    items
                        .into_iter()
                        .take(8)
                        .filter_map(validate_result)
                        .filter(|item| {
                            item.project_id == project
                                && item.worktree_id.is_none_or(|id| id == worktree)
                        }),
                );
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
        right
            .relevance
            .total_cmp(&left.relevance)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.source_uri.cmp(&right.source_uri))
    });
    results.truncate(32);
    results
}

fn valid_cache(
    state: &CircuitState,
    project_id: ProjectId,
    now: time::OffsetDateTime,
    ttl: Duration,
) -> Vec<ProviderResult> {
    let valid_time = state.cache_time.is_some_and(|cached| {
        now - cached <= time::Duration::try_from(ttl).unwrap_or(time::Duration::ZERO)
    });
    if valid_time {
        state
            .cache
            .iter()
            .filter(|item| item.project_id == project_id)
            .cloned()
            .collect()
    } else {
        Vec::new()
    }
}

fn validate_result(mut result: ProviderResult) -> Option<ProviderResult> {
    if result.provider.trim().is_empty()
        || result.title.trim().is_empty()
        || result.content.trim().is_empty()
        || result.source_uri.trim().is_empty()
        || result.trust.trim().is_empty()
        || !result.relevance.is_finite()
        || !(0.0..=1.0).contains(&result.relevance)
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

fn normalized(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn is_same_or_child(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

const fn enabled_by_default() -> bool {
    true
}

const fn default_codegraph_deadline_ms() -> u64 {
    5_000
}

const fn default_llm_wiki_deadline_ms() -> u64 {
    300
}

const fn default_llm_wiki_results() -> usize {
    3
}

const fn default_llm_wiki_tokens() -> usize {
    600
}

const fn default_cache_ttl_seconds() -> u64 {
    86_400
}

const fn default_relevance() -> f64 {
    0.5
}
