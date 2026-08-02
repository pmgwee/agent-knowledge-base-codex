use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_context::{ContextCompiler, ContextQuery, LiveState, token_count};
use brain_coordination::{CoordinationStore, Overlap, overlap};
use brain_domain::{Harness, HookEnvelope, HookReply, ProjectId, WorktreeId};
use brain_store::EventLedger;

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
        if !matches!(envelope.harness, Harness::ClaudeCode | Harness::Codex)
            || envelope.event_name != "SessionStart"
        {
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
        let mut query = ContextQuery::for_worktree(binding.project_id, binding.worktree_id);
        let coordination = coordination_context(&binding).ok().flatten();
        if coordination.is_some() {
            query.max_tokens = 1_000;
        }
        query.native_session_id = envelope
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let compiled = compiler.compile(query)?;
        if compiled.citations.is_empty() && coordination.is_none() {
            return Ok(HookReply::default());
        }
        let additional_context = match coordination {
            Some(coordination) => format!("{coordination}\n\n{}", compiled.text),
            None => compiled.text,
        };
        debug_assert!(token_count(&additional_context) <= 1_500);
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

fn coordination_context(binding: &HookProjectBinding) -> Result<Option<String>> {
    let store = CoordinationStore::open(&binding.ledger_path, binding.project_id)?;
    let tasks = store.tasks(false)?;
    if tasks.is_empty() {
        return Ok(Some(
            "Coordination state: no active task is selected. Before editing, create or select a brain task and use its separate worktree; do not assume writer ownership."
                .to_owned(),
        ));
    }
    let now = time::OffsetDateTime::now_utc();
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
