use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_context::{
    ContextCompiler, ContextQuery, LiveState, ProviderConfig, ProviderResult, token_count,
};
use brain_coordination::{
    CoordinationStore, LeaseError, Overlap, RENEWAL_INTERVAL, SessionIdentity, overlap,
};
use brain_domain::{Harness, HookEnvelope, HookReply, ProjectId, WorktreeId};
use brain_store::{EventLedger, ProviderCacheStore};

#[derive(Clone, Debug)]
pub struct HookProjectBinding {
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
    pub global_preferences_path: Option<PathBuf>,
}

pub struct ProjectHookHandler {
    bindings: Vec<ResolvedHookBinding>,
}

struct ResolvedHookBinding {
    binding: HookProjectBinding,
    normalized_root: String,
}

/// Backwards-compatible foundation name. New integrations should use
/// `ProjectHookHandler` because project boundaries and context are shared.
pub type ClaudeHookHandler = ProjectHookHandler;

impl ProjectHookHandler {
    pub fn new(binding: HookProjectBinding) -> Result<Self> {
        Self::for_projects(vec![binding])
    }

    pub fn for_projects(bindings: Vec<HookProjectBinding>) -> Result<Self> {
        let mut resolved = Vec::with_capacity(bindings.len());
        for mut binding in bindings {
            binding.project_root = canonical_project_root(&binding.project_root)?;
            resolved.push(ResolvedHookBinding {
                normalized_root: normalize_path(&binding.project_root),
                binding,
            });
        }
        resolved.sort_by_key(|binding| std::cmp::Reverse(binding.normalized_root.len()));
        Ok(Self { bindings: resolved })
    }

    pub fn handle(&self, envelope: &HookEnvelope) -> Result<HookReply> {
        if !matches!(
            envelope.harness,
            Harness::ClaudeCode | Harness::Codex | Harness::Hermes
        ) {
            return Ok(HookReply::default());
        }
        let Some(cwd) = envelope
            .payload
            .get("cwd")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(HookReply::default());
        };
        let Ok(canonical_cwd) = std::fs::canonicalize(cwd) else {
            return Ok(HookReply::default());
        };
        let normalized_cwd = normalize_path(&canonical_cwd);
        let Some(binding) = self.resolve_binding(&normalized_cwd) else {
            return Ok(HookReply::default());
        };
        let lease_warning = manage_lease_lifecycle(&binding, envelope)?;
        if envelope.event_name != "SessionStart" {
            return Ok(HookReply {
                additional_context: lease_warning,
                diagnostics_id: Some(envelope.nonce.to_string()),
            });
        }

        let ledger = EventLedger::open(&binding.ledger_path, binding.project_id)?;
        let mut compiler =
            ContextCompiler::from_ledger(&ledger, binding.project_id, 500)?.with_live_state(
                LiveState::inspect(&binding.project_root, binding.worktree_id),
            );
        if let Some(path) = binding
            .global_preferences_path
            .as_ref()
            .filter(|path| path.is_file())
        {
            let preferences =
                brain_store::GlobalPreferenceStore::open(path)?.current_preferences()?;
            compiler = compiler.with_global_preferences(preferences);
        }
        if let Ok(results) = cached_wiki_context(&binding, envelope.received_at)
            && !results.is_empty()
        {
            compiler = compiler.with_provider_results(results);
        }
        let mut query = ContextQuery::for_worktree(binding.project_id, binding.worktree_id);
        let coordination = coordination_context(&binding, envelope.received_at)
            .ok()
            .flatten();
        if coordination.is_some() {
            query.max_tokens = 1_000;
        }
        let native_session_id = envelope
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        query.native_session_id = native_session_id.clone();
        let compiled = compiler.compile(query)?;
        if compiled.citations.is_empty() && coordination.is_none() && lease_warning.is_none() {
            return Ok(HookReply::default());
        }
        let coordination = [lease_warning, coordination]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        // Measured before `compiled.text` is consumed below.
        let memory_tokens = token_count(&compiled.text) as u64;
        let coordination_tokens = token_count(&coordination) as u64;
        let citation_count = compiled.citations.len() as u64;
        let additional_context = if coordination.is_empty() {
            compiled.text
        } else {
            format!("{coordination}\n\n{}", compiled.text)
        };
        debug_assert!(token_count(&additional_context) <= 1_500);
        // What the brain handed over is not an event and appears in no transcript, so it is
        // recorded as it happens or not at all. Deliberately fail-open: losing a metric must
        // never cost a session its orientation.
        let _ = ledger.record_context_delivery(&brain_store::ContextDelivery {
            project_id: binding.project_id,
            harness: envelope.harness.clone(),
            native_session_id,
            event_name: envelope.event_name.to_string(),
            delivered_at: envelope.received_at,
            total_tokens: token_count(&additional_context) as u64,
            memory_tokens,
            coordination_tokens,
            citation_count,
        });
        Ok(HookReply {
            additional_context: Some(additional_context),
            diagnostics_id: Some(envelope.nonce.to_string()),
        })
    }

    fn resolve_binding(&self, normalized_cwd: &str) -> Option<HookProjectBinding> {
        if let Some(resolved) = self
            .bindings
            .iter()
            .find(|binding| is_within_root(normalized_cwd, &binding.normalized_root))
        {
            return Some(resolved.binding.clone());
        }
        for resolved in &self.bindings {
            let Ok(store) =
                CoordinationStore::open(&resolved.binding.ledger_path, resolved.binding.project_id)
            else {
                continue;
            };
            let Ok(tasks) = store.tasks(false) else {
                continue;
            };
            for task in tasks {
                let Some(path) = task.worktree_path.as_deref() else {
                    continue;
                };
                let resolved_path =
                    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                let normalized_task = normalize_path(&resolved_path);
                if is_within_root(normalized_cwd, &normalized_task) {
                    let mut binding = resolved.binding.clone();
                    binding.project_root = resolved_path;
                    binding.worktree_id = task.worktree_id;
                    return Some(binding);
                }
            }
        }
        None
    }
}

fn cached_wiki_context(
    binding: &HookProjectBinding,
    now: time::OffsetDateTime,
) -> Result<Vec<ProviderResult>> {
    let project_storage = binding
        .ledger_path
        .parent()
        .context("project ledger has no storage directory")?;
    let brain_home = project_storage
        .parent()
        .and_then(Path::parent)
        .context("project ledger is not under BRAIN_HOME/projects/<id>")?;
    let config = ProviderConfig::load(project_storage.join("providers.json"), brain_home)?;
    if !config.llm_wiki.enabled {
        return Ok(Vec::new());
    }
    let config_hash = config.sha256()?;
    let cache = ProviderCacheStore::open(&binding.ledger_path, binding.project_id)?;
    let Some(entry) = cache.latest("llm_wiki", config_hash, now)? else {
        return Ok(Vec::new());
    };
    let age = (now - entry.fetched_at).whole_seconds().max(0);
    let mut items = serde_json::from_value::<Vec<ProviderResult>>(entry.items)?;
    items.retain(|item| {
        item.project_id == binding.project_id
            && item.worktree_id.is_none_or(|id| id == binding.worktree_id)
            && !item.source_uri.trim().is_empty()
    });
    items.truncate(2);
    for item in &mut items {
        item.trust = format!("external_document_cached age={age}s");
    }
    Ok(items)
}

fn manage_lease_lifecycle(
    binding: &HookProjectBinding,
    envelope: &HookEnvelope,
) -> Result<Option<String>> {
    let Some(task_id) = envelope
        .payload
        .get("brain_task_id")
        .and_then(serde_json::Value::as_str)
        .map(uuid::Uuid::parse_str)
        .transpose()?
    else {
        return Ok(None);
    };
    let Some(native_session_id) = envelope
        .payload
        .get("session_id")
        .or_else(|| envelope.payload.get("sessionId"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(Some(
            "Writer lease was not acquired because the native session ID is missing.".to_owned(),
        ));
    };
    let owner = SessionIdentity {
        harness: envelope.harness.clone(),
        native_session_id: native_session_id.to_owned(),
    };
    let mut store = CoordinationStore::open(&binding.ledger_path, binding.project_id)?;
    let Some(task) = store.task(task_id)? else {
        return Ok(Some(format!(
            "Writer lease was not acquired because task {task_id} is not an active project task."
        )));
    };
    if task.worktree_id != binding.worktree_id {
        return Ok(Some(format!(
            "Writer lease was not acquired: task {task_id} belongs to worktree {}, but this session is in worktree {}. Switch to the task worktree before editing.",
            task.worktree_id.0, binding.worktree_id.0
        )));
    }
    let now = envelope.received_at;
    let result = match envelope.event_name.as_str() {
        "SessionEnd" | "session.end" => {
            let Some(lease) = store.lease(task_id)? else {
                return Ok(None);
            };
            store
                .release_lease(task_id, &owner, lease.generation, now)
                .map(|_| None)
        }
        "SessionStart" => store.acquire_lease(task_id, owner, now, None).map(|_| None),
        _ => match store.lease(task_id)? {
            Some(lease)
                if lease.owner == owner
                    && lease.renewed_at + RENEWAL_INTERVAL > now
                    && lease.expires_at > now =>
            {
                return Ok(None);
            }
            Some(lease) if lease.owner == owner && lease.expires_at > now => store
                .renew_lease(task_id, &owner, lease.generation, now, None)
                .map(|_| None),
            _ => store.acquire_lease(task_id, owner, now, None).map(|_| None),
        },
    };
    match result {
        Ok(value) => Ok(value),
        Err(LeaseError::AlreadyHeld {
            owner_harness,
            owner_session,
            expires_at,
        }) => Ok(Some(format!(
            "WARNING: writer lease is held by {owner_harness}/{owner_session} until {expires_at}. Do not edit in this worktree; create or switch to a separate task worktree, or perform an explicit handoff."
        ))),
        Err(error) => Ok(Some(format!("Writer lease update failed: {error}"))),
    }
}

pub(crate) fn coordination_context(
    binding: &HookProjectBinding,
    now: time::OffsetDateTime,
) -> Result<Option<String>> {
    let store = CoordinationStore::open(&binding.ledger_path, binding.project_id)?;
    let tasks = store.tasks(false)?;
    if tasks.is_empty() {
        return Ok(Some(
            "Coordination state: no active task is selected. Before editing, create or select a brain task and use its separate worktree; do not assume writer ownership."
                .to_owned(),
        ));
    }
    let leases = store.active_leases(now)?;
    let claims = store.active_claims()?;
    let mut lines =
        vec!["Coordination state (current project-scoped leases and claims):".to_owned()];
    for task in &tasks {
        let owner = leases
            .iter()
            .find(|lease| lease.task_id == task.id)
            .map(|lease| {
                format!(
                    "{} / {} until {} (generation {})",
                    lease.owner.harness.as_str(),
                    lease.owner.native_session_id,
                    lease.expires_at,
                    lease.generation
                )
            })
            .unwrap_or_else(|| "no active writer lease".to_owned());
        let current = if task.worktree_id == binding.worktree_id {
            " [current worktree]"
        } else {
            ""
        };
        lines.push(format!(
            "- task {} {:?}{}; owner={owner}",
            task.id, task.title, current
        ));
    }
    for claim in &claims {
        lines.push(format!(
            "- claim task={} kind={:?} path={}",
            claim.task_id, claim.kind, claim.display_value
        ));
    }
    for (index, left) in claims.iter().enumerate() {
        for right in claims.iter().skip(index + 1) {
            if left.task_id == right.task_id {
                continue;
            }
            let level = overlap(left, right);
            if matches!(
                level,
                Overlap::Definite | Overlap::Probable | Overlap::Possible
            ) {
                lines.push(format!(
                    "- OVERLAP {level:?}: task {} {} <> task {} {}",
                    left.task_id, left.display_value, right.task_id, right.display_value
                ));
            }
        }
    }
    Ok(Some(bound_tokens(&lines.join("\n"), 350)))
}

fn bound_tokens(value: &str, max_tokens: usize) -> String {
    if token_count(value) <= max_tokens {
        return value.to_owned();
    }
    let mut words = value.split_whitespace().collect::<Vec<_>>();
    while !words.is_empty() && token_count(&words.join(" ")) > max_tokens.saturating_sub(10) {
        words.pop();
    }
    words.join(" ") + " ... [coordination truncated]"
}

fn canonical_project_root(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("resolve hook project root {}", path.display()))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn is_within_root(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}
