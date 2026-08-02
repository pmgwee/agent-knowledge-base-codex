use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use brain_context::{
    CodeGraphProvider, CompiledContext, ContextCompiler, ContextQuery, LiveState, LlmWikiProvider,
    ProcessCodeGraphClient, ProviderConfig, ProviderResult, ProviderStatus, RankedCandidate,
    RetrievalEngine, RetrievalQuery,
};
use brain_coordination::{
    ClaimResult, CoordinationStore, MergePreflight, PathClaim, PathClaimInput, SessionIdentity,
    WriterLease, merge_preflight,
};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, ProjectRegistry, SourceCursor,
};
use brain_store::{
    EventLedger, GlobalPreferenceStore, ProviderCacheEntry, ProviderCacheStore, SearchSource,
    SearchSourceFilter,
};
use sha2::{Digest, Sha256};

use crate::{ServiceLaunchConfig, ServiceProjectConfig};

const DEFAULT_RESULT_LIMIT: usize = 20;
const MAX_RESULT_LIMIT: usize = 100;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainSearchRequest {
    pub project: String,
    pub text: String,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub worktree_id: Option<brain_domain::WorktreeId>,
    #[serde(default)]
    pub task_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub native_session_id: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub source: SourceSelector,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainTimelineRequest {
    pub project: String,
    #[serde(default)]
    pub window: TimelineWindow,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub now: Option<String>,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub source: SourceSelector,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainCheckpointRequest {
    pub project: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainPromptContextRequest {
    pub project: String,
    pub harness: Harness,
    pub native_session_id: String,
    pub prompt: String,
    #[serde(default)]
    pub task_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainProviderState {
    pub provider: String,
    pub status: ProviderStatus,
    pub item_count: usize,
    pub latency_ms: u64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainPromptContextResponse {
    pub project_id: ProjectId,
    pub first_prompt_claimed: bool,
    pub context: CompiledContext,
    pub providers: Vec<BrainProviderState>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainEvidenceRequest {
    pub project: String,
    pub reference: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainCorrectionRequest {
    pub project: String,
    pub correction_id: uuid::Uuid,
    #[serde(default)]
    pub memory_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub kind: Option<MemoryKind>,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub evidence_ids: Vec<uuid::Uuid>,
    #[serde(default)]
    pub valid_from: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainStatusRequest {
    pub project: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainClaimRequest {
    pub project: String,
    pub task_id: uuid::Uuid,
    pub claims: Vec<PathClaimInput>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainClaimsRequest {
    pub project: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainReleaseClaimRequest {
    pub project: String,
    pub task_id: uuid::Uuid,
    pub claim_id: uuid::Uuid,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainClaimsResponse {
    pub project_id: ProjectId,
    pub claims: Vec<PathClaim>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainLeaseAcquireRequest {
    pub project: String,
    pub task_id: uuid::Uuid,
    pub owner: SessionIdentity,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainLeaseGenerationRequest {
    pub project: String,
    pub task_id: uuid::Uuid,
    pub owner: SessionIdentity,
    pub generation: u64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainLeaseHandoffRequest {
    pub project: String,
    pub handoff_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub current_owner: SessionIdentity,
    pub generation: u64,
    pub next_owner: SessionIdentity,
    pub checkpoint: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainLeasesRequest {
    pub project: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainLeaseResponse {
    pub project_id: ProjectId,
    pub lease: Option<WriterLease>,
    pub leases: Vec<WriterLease>,
    pub replayed: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainPreflightRequest {
    pub project: String,
    #[serde(default)]
    pub task_id: Option<uuid::Uuid>,
    #[serde(default = "default_source_ref")]
    pub source_ref: String,
    pub target_ref: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainPreflightResponse {
    pub project_id: ProjectId,
    pub result: MergePreflight,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainClaimResponse {
    pub project_id: ProjectId,
    pub results: Vec<ClaimResult>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSelector {
    #[default]
    All,
    Events,
    Memories,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineWindow {
    Day,
    #[default]
    Week,
    Month,
    Custom,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainItemsResponse {
    pub project_id: ProjectId,
    pub items: Vec<BrainResultItem>,
    pub truncated: bool,
    pub optional_provider_state: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainResultItem {
    pub reference: String,
    pub source: String,
    pub memory_id: Option<uuid::Uuid>,
    pub harness: Option<Harness>,
    pub kind: Option<MemoryKind>,
    pub title: String,
    pub text: String,
    pub path: Option<String>,
    pub occurred_at: String,
    pub observed_at: String,
    pub temporal_role: String,
    pub late_observation: bool,
    pub authority: Option<Authority>,
    pub status: Option<MemoryStatus>,
    pub score: f64,
    pub ranking_reasons: Vec<String>,
    pub citations: Vec<BrainCitation>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainCitation {
    pub reference: String,
    pub source: String,
    pub observed_at: String,
    pub evidence_ids: Vec<uuid::Uuid>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainCheckpointResponse {
    pub project_id: ProjectId,
    pub context: CompiledContext,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainEvidenceResponse {
    pub project_id: ProjectId,
    pub reference: String,
    pub source: String,
    pub record: serde_json::Value,
    pub citations: Vec<BrainCitation>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainCorrectionResponse {
    pub project_id: ProjectId,
    pub memory: MemoryRecord,
    pub audit_event_id: uuid::Uuid,
    pub replayed: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct BrainStatusResponse {
    pub project_id: ProjectId,
    pub project_root: PathBuf,
    pub event_count: u64,
    pub memory_count: u64,
    pub memory_version_count: u64,
    pub active_schema_drifts: u64,
    pub source_count: usize,
    pub latest_event_at: Option<String>,
    pub canonical_retrieval: String,
    pub optional_providers: String,
}

pub struct BrainQueryService {
    brain_home: PathBuf,
    config: ServiceLaunchConfig,
}

impl BrainQueryService {
    pub fn open(brain_home: impl AsRef<Path>) -> Result<Self> {
        let brain_home = brain_home.as_ref().to_path_buf();
        let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
        Ok(Self { brain_home, config })
    }

    pub fn from_config(brain_home: impl AsRef<Path>, config: ServiceLaunchConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            brain_home: brain_home.as_ref().to_path_buf(),
            config,
        })
    }

    pub fn search(&self, request: BrainSearchRequest) -> Result<BrainItemsResponse> {
        ensure!(!request.text.trim().is_empty(), "search text is required");
        let project = self.project(&request.project)?;
        let ledger = self.ledger(project)?;
        let now = time::OffsetDateTime::now_utc();
        let mut query = RetrievalQuery::text(project.project_id, request.text, now)
            .with_paths(request.paths)
            .with_limit(bounded_limit(request.limit).saturating_add(1));
        query.as_of = request.as_of.as_deref().map(parse_timestamp).transpose()?;
        query.worktree_id = request.worktree_id;
        query.task_id = request.task_id;
        query.native_session_id = request.native_session_id;
        query.source_filter = request.source.into();
        self.retrieve(&ledger, query, bounded_limit(request.limit))
    }

    pub fn timeline(&self, request: BrainTimelineRequest) -> Result<BrainItemsResponse> {
        let project = self.project(&request.project)?;
        let ledger = self.ledger(project)?;
        let now = request
            .now
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .unwrap_or_else(time::OffsetDateTime::now_utc);
        let (start, end) = match request.window {
            TimelineWindow::Day => (now - time::Duration::days(1), now),
            TimelineWindow::Week => (now - time::Duration::days(7), now),
            TimelineWindow::Month => (now - time::Duration::days(30), now),
            TimelineWindow::Custom => (
                parse_timestamp(
                    request
                        .start
                        .as_deref()
                        .context("custom timeline requires start")?,
                )?,
                parse_timestamp(
                    request
                        .end
                        .as_deref()
                        .context("custom timeline requires end")?,
                )?,
            ),
        };
        ensure!(start < end, "timeline start must precede end");
        let mut query = RetrievalQuery::between(project.project_id, start, end, now)
            .with_limit(bounded_limit(request.limit).saturating_add(1));
        query.as_of = request.as_of.as_deref().map(parse_timestamp).transpose()?;
        query.source_filter = request.source.into();
        self.retrieve(&ledger, query, bounded_limit(request.limit))
    }

    pub fn checkpoint(&self, request: BrainCheckpointRequest) -> Result<BrainCheckpointResponse> {
        let project = self.project(&request.project)?;
        self.compile_checkpoint(
            project,
            &project.project_root,
            project.worktree_id,
            request,
            Vec::new(),
        )
    }

    pub fn context_for_prompt(
        &self,
        request: BrainPromptContextRequest,
    ) -> Result<BrainPromptContextResponse> {
        ensure!(
            !request.native_session_id.trim().is_empty(),
            "native session ID is required"
        );
        ensure!(
            !request.prompt.trim().is_empty(),
            "first prompt is required"
        );
        let project = self.project(&request.project)?.clone();
        let (worktree_root, worktree_id) = if let Some(task_id) = request.task_id {
            let task = CoordinationStore::open(&project.ledger_path, project.project_id)?
                .task(task_id)?
                .context("prompt task does not exist")?;
            (
                task.worktree_path
                    .context("prompt task has no validated worktree")?,
                task.worktree_id,
            )
        } else {
            (project.project_root.clone(), project.worktree_id)
        };
        let project_storage = project
            .ledger_path
            .parent()
            .context("project ledger has no storage directory")?;
        let config_path = project_storage.join("providers.json");
        let config = ProviderConfig::load(config_path, &self.brain_home)?;
        let config_hash = config.sha256()?;
        let now = time::OffsetDateTime::now_utc();
        let prompt_hash: [u8; 32] = Sha256::digest(
            [
                request.prompt.as_bytes(),
                &[0],
                request.paths.join("\n").as_bytes(),
            ]
            .concat(),
        )
        .into();
        let mut cache = ProviderCacheStore::open(&project.ledger_path, project.project_id)?;
        let first_prompt_claimed = cache.claim_first_prompt(
            &request.harness,
            &request.native_session_id,
            prompt_hash,
            now,
        )?;
        let mut query = ContextQuery::for_worktree(project.project_id, worktree_id);
        query.native_session_id = Some(request.native_session_id.clone());
        query.prompt = Some(request.prompt.clone());
        query.paths = request.paths.clone();
        if let Some(max_tokens) = request.max_tokens {
            query.max_tokens = max_tokens;
        }
        let mut provider_results = Vec::new();
        let mut provider_states = Vec::new();

        if config.llm_wiki.enabled {
            let started = std::time::Instant::now();
            match config.llm_wiki.vault.as_ref() {
                Some(vault) => match LlmWikiProvider::new(
                    project.project_id,
                    worktree_id,
                    vault,
                    &self.brain_home,
                    std::time::Duration::from_millis(config.llm_wiki.deadline_ms),
                    config.llm_wiki.max_results,
                    config.llm_wiki.max_tokens,
                ) {
                    Ok(provider) => {
                        let source_version = provider.source_version()?;
                        let cached = cache.get(
                            "llm_wiki",
                            prompt_hash,
                            config_hash,
                            &source_version,
                            now,
                        )?;
                        let (items, status, reason) = if let Some(entry) = cached {
                            (
                                serde_json::from_value::<Vec<ProviderResult>>(entry.items)?,
                                ProviderStatus::Ready,
                                Some("served idempotent project-scoped cache".to_owned()),
                            )
                        } else if first_prompt_claimed {
                            match provider.search_blocking(&query) {
                                Ok(items) => {
                                    cache.put(&ProviderCacheEntry {
                                        provider: "llm_wiki".to_owned(),
                                        project_id: project.project_id,
                                        task_id: request.task_id,
                                        query_sha256: prompt_hash,
                                        config_sha256: config_hash,
                                        source_version,
                                        fetched_at: now,
                                        expires_at: now
                                            + time::Duration::seconds(i64::try_from(
                                                config.llm_wiki.cache_ttl_seconds,
                                            )?),
                                        items: serde_json::to_value(&items)?,
                                    })?;
                                    (items, ProviderStatus::Ready, None)
                                }
                                Err(error) => {
                                    (Vec::new(), ProviderStatus::Failed, Some(error.to_string()))
                                }
                            }
                        } else {
                            (
                                Vec::new(),
                                ProviderStatus::Disabled,
                                Some(
                                    "live LLM Wiki search already consumed for this session"
                                        .to_owned(),
                                ),
                            )
                        };
                        provider_states.push(BrainProviderState {
                            provider: "llm_wiki".to_owned(),
                            status,
                            item_count: items.len(),
                            latency_ms: elapsed_ms(started),
                            reason,
                        });
                        provider_results.extend(items);
                    }
                    Err(error) => provider_states.push(BrainProviderState {
                        provider: "llm_wiki".to_owned(),
                        status: ProviderStatus::Unsupported,
                        item_count: 0,
                        latency_ms: elapsed_ms(started),
                        reason: Some(error.to_string()),
                    }),
                },
                None => provider_states.push(BrainProviderState {
                    provider: "llm_wiki".to_owned(),
                    status: ProviderStatus::Unsupported,
                    item_count: 0,
                    latency_ms: elapsed_ms(started),
                    reason: Some("no reviewed Markdown vault is configured".to_owned()),
                }),
            }
        } else {
            provider_states.push(disabled_state("llm_wiki"));
        }

        if config.codegraph.enabled {
            let started = std::time::Instant::now();
            let result = (|| {
                let executable = config
                    .codegraph
                    .executable
                    .as_ref()
                    .context("CodeGraph executable is not configured")?;
                ensure!(
                    config.codegraph.activation_report_sha256.is_some(),
                    "CodeGraph activation benchmark has not passed"
                );
                let head = git_head(&worktree_root)?;
                let client = std::sync::Arc::new(ProcessCodeGraphClient::with_deadline(
                    executable,
                    std::time::Duration::from_millis(config.codegraph.deadline_ms),
                )?);
                let provider = CodeGraphProvider::new(
                    client,
                    project.project_id,
                    worktree_id,
                    &worktree_root,
                    head,
                    true,
                )?;
                provider.search_blocking(&query)
            })();
            match result {
                Ok(items) => {
                    provider_states.push(BrainProviderState {
                        provider: "codegraph".to_owned(),
                        status: ProviderStatus::Ready,
                        item_count: items.len(),
                        latency_ms: elapsed_ms(started),
                        reason: None,
                    });
                    provider_results.extend(items);
                }
                Err(error) => provider_states.push(BrainProviderState {
                    provider: "codegraph".to_owned(),
                    status: ProviderStatus::Stale,
                    item_count: 0,
                    latency_ms: elapsed_ms(started),
                    reason: Some(error.to_string()),
                }),
            }
        } else {
            provider_states.push(disabled_state("codegraph"));
        }

        let checkpoint = self.compile_checkpoint(
            &project,
            &worktree_root,
            worktree_id,
            BrainCheckpointRequest {
                project: request.project,
                prompt: Some(request.prompt),
                paths: request.paths,
                as_of: None,
                max_tokens: request.max_tokens,
            },
            provider_results,
        )?;
        Ok(BrainPromptContextResponse {
            project_id: project.project_id,
            first_prompt_claimed,
            context: checkpoint.context,
            providers: provider_states,
        })
    }

    fn compile_checkpoint(
        &self,
        project: &ServiceProjectConfig,
        worktree_root: &Path,
        worktree_id: brain_domain::WorktreeId,
        request: BrainCheckpointRequest,
        provider_results: Vec<ProviderResult>,
    ) -> Result<BrainCheckpointResponse> {
        let ledger = self.ledger(project)?;
        let mut compiler = ContextCompiler::from_ledger(&ledger, project.project_id, 500)?
            .with_live_state(LiveState::inspect(worktree_root, worktree_id));
        let global_path = self
            .brain_home
            .join("global-preferences")
            .join("preferences.sqlite");
        if global_path.is_file() {
            compiler = compiler.with_global_preferences(
                GlobalPreferenceStore::open(global_path)?.current_preferences()?,
            );
        }
        if !provider_results.is_empty() {
            compiler = compiler.with_provider_results(provider_results);
        }
        let mut query = ContextQuery::for_worktree(project.project_id, worktree_id);
        query.prompt = request.prompt;
        query.paths = request.paths;
        query.as_of = request.as_of.as_deref().map(parse_timestamp).transpose()?;
        if let Some(max_tokens) = request.max_tokens {
            query.max_tokens = max_tokens;
        }
        Ok(BrainCheckpointResponse {
            project_id: project.project_id,
            context: compiler.compile(query)?,
        })
    }

    pub fn evidence(&self, request: BrainEvidenceRequest) -> Result<BrainEvidenceResponse> {
        let project = self.project(&request.project)?;
        let ledger = self.ledger(project)?;
        let raw_id = request
            .reference
            .rsplit_once(':')
            .map_or(request.reference.as_str(), |(_, id)| id);
        let id = uuid::Uuid::parse_str(raw_id).context("evidence reference must contain a UUID")?;
        if let Some(event) = ledger.event(id)? {
            let citation = BrainCitation {
                reference: format!("event:{}", event.event_id),
                source: event.harness.as_str().to_owned(),
                observed_at: format_timestamp(event.observed_at),
                evidence_ids: vec![event.event_id],
            };
            return Ok(BrainEvidenceResponse {
                project_id: project.project_id,
                reference: citation.reference.clone(),
                source: "event".to_owned(),
                record: serde_json::json!({
                    "event_type": event.event_type,
                    "harness": event.harness,
                    "native_session_id": event.native_session_id,
                    "occurred_at": format_timestamp(event.occurred_at),
                    "observed_at": format_timestamp(event.observed_at),
                    "git_head": event.git_head,
                    "git_branch": event.git_branch,
                    "payload": bounded_json(&event.payload, 8_000),
                }),
                citations: vec![citation],
            });
        }
        let memory = ledger
            .memory_version(id)?
            .or_else(|| ledger.current_memory(id).ok().flatten())
            .with_context(|| format!("evidence {id} was not found in the selected project"))?;
        let citation = memory_citation(&memory);
        Ok(BrainEvidenceResponse {
            project_id: project.project_id,
            reference: format!("memory:{}", memory.version_id),
            source: "memory".to_owned(),
            record: serde_json::to_value(&memory)?,
            citations: vec![citation],
        })
    }

    pub fn correct(&self, request: BrainCorrectionRequest) -> Result<BrainCorrectionResponse> {
        ensure!(
            !request.title.trim().is_empty(),
            "correction title is required"
        );
        ensure!(
            !request.content.trim().is_empty(),
            "correction content is required"
        );
        let project = self.project(&request.project)?.clone();
        let mut ledger = self.ledger(&project)?;
        for evidence_id in &request.evidence_ids {
            ensure!(
                ledger.event(*evidence_id)?.is_some(),
                "evidence {evidence_id} does not belong to the selected project"
            );
        }
        let memory_id = request.memory_id.unwrap_or_else(|| {
            stable_uuid(&[b"mcp-correction-memory", request.correction_id.as_bytes()])
        });
        let version_id =
            stable_uuid(&[b"mcp-correction-version", request.correction_id.as_bytes()]);
        if let Some(existing) = ledger.memory_version(version_id)? {
            return Ok(BrainCorrectionResponse {
                project_id: project.project_id,
                memory: existing,
                audit_event_id: request.correction_id,
                replayed: true,
            });
        }
        let current = ledger.current_memory(memory_id)?;
        let kind = match (&current, request.kind) {
            (Some(current), Some(kind)) => {
                ensure!(
                    current.kind == kind,
                    "a correction cannot change memory kind"
                );
                kind
            }
            (Some(current), None) => current.kind.clone(),
            (None, Some(kind)) => kind,
            (None, None) => bail!("kind is required when creating a new corrected memory"),
        };
        ensure!(
            kind != MemoryKind::Preference,
            "global preferences require the explicit global-preference promotion path"
        );
        let observed_at = time::OffsetDateTime::now_utc();
        let valid_from = request
            .valid_from
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .unwrap_or(observed_at);
        let payload = serde_json::json!({
            "human_correction": true,
            "correction_id": request.correction_id,
            "memory_id": memory_id,
            "kind": kind,
            "title": request.title,
            "content": request.content,
            "supersedes": current.as_ref().map(|memory| memory.version_id),
        });
        let raw_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&payload)?).into();
        let idempotency_key: [u8; 32] = Sha256::digest(
            [
                b"mcp-correction".as_slice(),
                request.correction_id.as_bytes(),
            ]
            .concat(),
        )
        .into();
        ledger.append_batch(&EventBatch {
            source_id: format!("mcp-correction:{}", request.correction_id),
            events: vec![NormalizedEvent {
                event_id: request.correction_id,
                project_id: project.project_id,
                worktree_id: project.worktree_id,
                task_id: None,
                harness: Harness::Other("human-correction".to_owned()),
                native_session_id: format!("mcp-correction:{}", request.correction_id),
                native_turn_id: None,
                event_type: EventType::CheckpointAuthored,
                occurred_at: valid_from,
                observed_at,
                source_locator: format!("mcp://correction/{}", request.correction_id),
                source_offset: 1,
                source_schema: "brain-correction:v1".to_owned(),
                raw_hash,
                idempotency_key,
                git_head: None,
                git_branch: None,
                payload: payload.clone(),
                raw: payload,
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })?;
        let mut evidence_ids = request.evidence_ids;
        evidence_ids.push(request.correction_id);
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        let memory = MemoryRecord {
            id: memory_id,
            version_id,
            scope: MemoryScope::Project(project.project_id),
            worktree_id: Some(project.worktree_id),
            task_id: None,
            kind,
            title: request.title,
            content: request.content,
            valid_from,
            valid_to: None,
            recorded_at: observed_at,
            confidence: 1.0,
            authority: Authority::HumanCorrection,
            evidence_ids,
            supersedes: current
                .map(|memory| vec![memory.version_id])
                .unwrap_or_default(),
            status: MemoryStatus::Current,
        };
        ledger.append_memory(&memory)?;
        Ok(BrainCorrectionResponse {
            project_id: project.project_id,
            memory,
            audit_event_id: request.correction_id,
            replayed: false,
        })
    }

    pub fn status(&self, request: BrainStatusRequest) -> Result<BrainStatusResponse> {
        let project = self.project(&request.project)?;
        let ledger = self.ledger(project)?;
        Ok(BrainStatusResponse {
            project_id: project.project_id,
            project_root: project.project_root.clone(),
            event_count: ledger.event_count()?,
            memory_count: ledger.memory_count()?,
            memory_version_count: ledger.memory_version_count()?,
            active_schema_drifts: ledger.active_schema_drift_count()?,
            source_count: project.claude_sources.len()
                + project.codex_sources.len()
                + usize::from(project.hermes_database.is_some()),
            latest_event_at: ledger.latest_event_at()?.map(format_timestamp),
            canonical_retrieval: "sqlite_fts5_available".to_owned(),
            optional_providers: "not_required_for_canonical_retrieval".to_owned(),
        })
    }

    pub fn claim(&self, request: BrainClaimRequest) -> Result<BrainClaimResponse> {
        let project = self.project(&request.project)?.clone();
        let mut store = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        Ok(BrainClaimResponse {
            project_id: project.project_id,
            results: store.claim_paths(
                request.task_id,
                request.claims,
                time::OffsetDateTime::now_utc(),
            )?,
        })
    }

    pub fn claims(&self, request: BrainClaimsRequest) -> Result<BrainClaimsResponse> {
        let project = self.project(&request.project)?;
        Ok(BrainClaimsResponse {
            project_id: project.project_id,
            claims: CoordinationStore::open(&project.ledger_path, project.project_id)?
                .active_claims()?,
        })
    }

    pub fn release_claim(&self, request: BrainReleaseClaimRequest) -> Result<BrainClaimsResponse> {
        let project = self.project(&request.project)?.clone();
        let mut store = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        store.release_claim(
            request.claim_id,
            request.task_id,
            time::OffsetDateTime::now_utc(),
        )?;
        Ok(BrainClaimsResponse {
            project_id: project.project_id,
            claims: store.active_claims()?,
        })
    }

    pub fn acquire_lease(&self, request: BrainLeaseAcquireRequest) -> Result<BrainLeaseResponse> {
        let project = self.project(&request.project)?.clone();
        let mut store = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        let lease = store
            .acquire_lease(
                request.task_id,
                request.owner,
                time::OffsetDateTime::now_utc(),
                None,
            )
            .map_err(anyhow::Error::from)?;
        Ok(BrainLeaseResponse {
            project_id: project.project_id,
            lease: Some(lease),
            leases: store.active_leases(time::OffsetDateTime::now_utc())?,
            replayed: false,
        })
    }

    pub fn renew_lease(&self, request: BrainLeaseGenerationRequest) -> Result<BrainLeaseResponse> {
        let project = self.project(&request.project)?.clone();
        let mut store = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        let lease = store
            .renew_lease(
                request.task_id,
                &request.owner,
                request.generation,
                time::OffsetDateTime::now_utc(),
                None,
            )
            .map_err(anyhow::Error::from)?;
        Ok(BrainLeaseResponse {
            project_id: project.project_id,
            lease: Some(lease),
            leases: store.active_leases(time::OffsetDateTime::now_utc())?,
            replayed: false,
        })
    }

    pub fn release_lease(
        &self,
        request: BrainLeaseGenerationRequest,
    ) -> Result<BrainLeaseResponse> {
        let project = self.project(&request.project)?.clone();
        let mut store = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        store
            .release_lease(
                request.task_id,
                &request.owner,
                request.generation,
                time::OffsetDateTime::now_utc(),
            )
            .map_err(anyhow::Error::from)?;
        Ok(BrainLeaseResponse {
            project_id: project.project_id,
            lease: None,
            leases: store.active_leases(time::OffsetDateTime::now_utc())?,
            replayed: false,
        })
    }

    pub fn handoff_lease(&self, request: BrainLeaseHandoffRequest) -> Result<BrainLeaseResponse> {
        ensure!(
            !request.checkpoint.trim().is_empty(),
            "handoff checkpoint is required"
        );
        let project = self.project(&request.project)?.clone();
        let mut coordination = CoordinationStore::open(&project.ledger_path, project.project_id)?;
        if let Some(existing) = coordination.lease(request.task_id)?
            && existing.owner == request.next_owner
            && existing.generation == request.generation.saturating_add(1)
        {
            return Ok(BrainLeaseResponse {
                project_id: project.project_id,
                lease: Some(existing),
                leases: coordination.active_leases(time::OffsetDateTime::now_utc())?,
                replayed: true,
            });
        }
        let now = time::OffsetDateTime::now_utc();
        let payload = serde_json::json!({
            "summary": request.checkpoint,
            "handoff_id": request.handoff_id,
            "task_id": request.task_id,
            "from": request.current_owner,
            "to": request.next_owner,
        });
        let raw_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&payload)?).into();
        let idempotency_key: [u8; 32] =
            Sha256::digest([b"lease-handoff".as_slice(), request.handoff_id.as_bytes()].concat())
                .into();
        let mut ledger = self.ledger(&project)?;
        ledger.append_batch(&EventBatch {
            source_id: format!("lease-handoff:{}", request.handoff_id),
            events: vec![NormalizedEvent {
                event_id: request.handoff_id,
                project_id: project.project_id,
                worktree_id: coordination
                    .task(request.task_id)?
                    .context("handoff task does not exist")?
                    .worktree_id,
                task_id: Some(request.task_id),
                harness: request.current_owner.harness.clone(),
                native_session_id: request.current_owner.native_session_id.clone(),
                native_turn_id: None,
                event_type: EventType::CheckpointAuthored,
                occurred_at: now,
                observed_at: now,
                source_locator: format!("brain://handoff/{}", request.handoff_id),
                source_offset: 1,
                source_schema: "brain-handoff:v1".to_owned(),
                raw_hash,
                idempotency_key,
                git_head: None,
                git_branch: None,
                payload: payload.clone(),
                raw: payload,
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::byte_offset(1),
        })?;
        let lease = coordination
            .handoff_lease(
                request.task_id,
                &request.current_owner,
                request.generation,
                request.next_owner,
                now,
            )
            .map_err(anyhow::Error::from)?;
        Ok(BrainLeaseResponse {
            project_id: project.project_id,
            lease: Some(lease),
            leases: coordination.active_leases(now)?,
            replayed: false,
        })
    }

    pub fn leases(&self, request: BrainLeasesRequest) -> Result<BrainLeaseResponse> {
        let project = self.project(&request.project)?;
        Ok(BrainLeaseResponse {
            project_id: project.project_id,
            lease: None,
            leases: CoordinationStore::open(&project.ledger_path, project.project_id)?
                .active_leases(time::OffsetDateTime::now_utc())?,
            replayed: false,
        })
    }

    pub fn preflight(&self, request: BrainPreflightRequest) -> Result<BrainPreflightResponse> {
        let project = self.project(&request.project)?;
        let repository = if let Some(task_id) = request.task_id {
            CoordinationStore::open(&project.ledger_path, project.project_id)?
                .task(task_id)?
                .context("preflight task does not exist")?
                .worktree_path
                .context("preflight task has no validated worktree path")?
        } else {
            project.project_root.clone()
        };
        Ok(BrainPreflightResponse {
            project_id: project.project_id,
            result: merge_preflight(repository, &request.source_ref, &request.target_ref)?,
        })
    }

    fn retrieve(
        &self,
        ledger: &EventLedger,
        query: RetrievalQuery,
        limit: usize,
    ) -> Result<BrainItemsResponse> {
        let project_id = query.project_id;
        let mut candidates = RetrievalEngine::retrieve(ledger, &query)?;
        let truncated = candidates.len() > limit;
        candidates.truncate(limit);
        let items = candidates
            .into_iter()
            .map(|candidate| self.result_item(ledger, candidate, query.as_of.is_some()))
            .collect::<Result<Vec<_>>>()?;
        Ok(BrainItemsResponse {
            project_id,
            items,
            truncated,
            optional_provider_state: "canonical_fts_only".to_owned(),
        })
    }

    fn result_item(
        &self,
        ledger: &EventLedger,
        candidate: RankedCandidate,
        as_of: bool,
    ) -> Result<BrainResultItem> {
        let score = candidate.total_score();
        let hit = candidate.hit;
        let (source, reference, citations, temporal_role) = match hit.source {
            SearchSource::Event => (
                "event".to_owned(),
                format!("event:{}", hit.source_id),
                vec![BrainCitation {
                    reference: format!("event:{}", hit.source_id),
                    source: hit
                        .harness
                        .as_ref()
                        .map_or("unknown", Harness::as_str)
                        .to_owned(),
                    observed_at: format_timestamp(hit.observed_at),
                    evidence_ids: vec![hit.source_id],
                }],
                "historical_evidence".to_owned(),
            ),
            SearchSource::Memory => {
                let memory = ledger
                    .memory_version(hit.source_id)?
                    .context("ranked memory version disappeared")?;
                (
                    "memory".to_owned(),
                    format!("memory:{}", hit.source_id),
                    vec![memory_citation(&memory)],
                    if as_of {
                        "as_of_memory".to_owned()
                    } else {
                        "current_memory".to_owned()
                    },
                )
            }
        };
        Ok(BrainResultItem {
            reference,
            source,
            memory_id: hit.memory_id,
            harness: hit.harness,
            kind: hit.kind,
            title: hit.title,
            text: hit.text,
            path: hit.path,
            occurred_at: format_timestamp(hit.occurred_at),
            observed_at: format_timestamp(hit.observed_at),
            temporal_role,
            late_observation: hit.late_observation,
            authority: hit.authority,
            status: hit.status,
            score,
            ranking_reasons: candidate.reasons.into_iter().map(str::to_owned).collect(),
            citations,
        })
    }

    fn project(&self, selector: &str) -> Result<&ServiceProjectConfig> {
        ensure!(!selector.trim().is_empty(), "project is required");
        let registry = ProjectRegistry::open(&self.brain_home)?;
        let project_id = registry.resolve(selector)?;
        self.config.project(Some(project_id))
    }

    fn ledger(&self, project: &ServiceProjectConfig) -> Result<EventLedger> {
        EventLedger::open(&project.ledger_path, project.project_id)
    }
}

impl From<SourceSelector> for SearchSourceFilter {
    fn from(value: SourceSelector) -> Self {
        match value {
            SourceSelector::All => Self::All,
            SourceSelector::Events => Self::Events,
            SourceSelector::Memories => Self::Memories,
        }
    }
}

fn bounded_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_RESULT_LIMIT).min(MAX_RESULT_LIMIT)
}

fn parse_timestamp(value: &str) -> Result<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .with_context(|| format!("invalid RFC3339 timestamp {value:?}"))
}

fn memory_citation(memory: &MemoryRecord) -> BrainCitation {
    BrainCitation {
        reference: format!("memory:{}", memory.version_id),
        source: memory.authority.as_str().to_owned(),
        observed_at: format_timestamp(memory.recorded_at),
        evidence_ids: memory.evidence_ids.clone(),
    }
}

fn bounded_json(value: &serde_json::Value, max_chars: usize) -> String {
    let rendered = value.to_string();
    if rendered.chars().count() <= max_chars {
        rendered
    } else {
        rendered.chars().take(max_chars).collect::<String>() + "... [truncated]"
    }
}

fn format_timestamp(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| value.to_string())
}

fn stable_uuid(parts: &[&[u8]]) -> uuid::Uuid {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

fn default_source_ref() -> String {
    "HEAD".to_owned()
}

fn disabled_state(provider: &str) -> BrainProviderState {
    BrainProviderState {
        provider: provider.to_owned(),
        status: ProviderStatus::Disabled,
        item_count: 0,
        latency_ms: 0,
        reason: Some("provider is disabled for this project".to_owned()),
    }
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn git_head(root: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()?;
    ensure!(
        output.status.success(),
        "resolve current Git HEAD: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
