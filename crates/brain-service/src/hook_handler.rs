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

pub struct ClaudeHookHandler {
    binding: HookProjectBinding,
    normalized_root: String,
}

impl ClaudeHookHandler {
    pub fn new(mut binding: HookProjectBinding) -> Result<Self> {
        binding.project_root = std::fs::canonicalize(&binding.project_root).with_context(|| {
            format!(
                "resolve hook project root {}",
                binding.project_root.display()
            )
        })?;
        let normalized_root = normalize_path(&binding.project_root);
        Ok(Self {
            binding,
            normalized_root,
        })
    }

    pub fn handle(&self, envelope: &HookEnvelope) -> Result<HookReply> {
        if envelope.harness != Harness::ClaudeCode || envelope.event_name != "SessionStart" {
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
        if !is_within_root(&normalize_path(&canonical_cwd), &self.normalized_root) {
            return Ok(HookReply::default());
        }

        let ledger = EventLedger::open(&self.binding.ledger_path, self.binding.project_id)?;
        let compiler = ContextCompiler::from_ledger(&ledger, self.binding.project_id, 500)?;
        let mut query =
            ContextQuery::for_worktree(self.binding.project_id, self.binding.worktree_id);
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

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn is_within_root(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}
