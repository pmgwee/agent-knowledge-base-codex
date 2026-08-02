use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_context::{ContextCompiler, ContextQuery};
use brain_domain::{Harness, HookEnvelope, HookReply, ProjectId, WorktreeId};
use brain_store::EventLedger;

#[derive(Clone, Debug)]
pub struct HookProjectBinding {
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
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
        let Some(resolved) = self
            .bindings
            .iter()
            .find(|binding| is_within_root(&normalized_cwd, &binding.normalized_root))
        else {
            return Ok(HookReply::default());
        };
        let binding = &resolved.binding;

        let ledger = EventLedger::open(&binding.ledger_path, binding.project_id)?;
        let compiler = ContextCompiler::from_ledger(&ledger, binding.project_id, 500)?;
        let mut query = ContextQuery::for_worktree(binding.project_id, binding.worktree_id);
        query.native_session_id = envelope
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let compiled = compiler.compile(query)?;
        if compiled.evidence_ids.is_empty() {
            return Ok(HookReply::default());
        }
        Ok(HookReply {
            additional_context: Some(compiled.text),
            diagnostics_id: Some(envelope.nonce.to_string()),
        })
    }
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
