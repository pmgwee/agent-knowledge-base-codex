use std::sync::Arc;

use anyhow::Result;
use brain_adapters::{
    ClaudeAdapter, CodexAdapter, HermesAdapter, NormalizeContext, SourceDescriptor,
};

use crate::{CaptureBinding, HookProjectBinding, ServiceLaunchConfig};

pub fn build_capture_bindings(config: &ServiceLaunchConfig) -> Result<Vec<CaptureBinding>> {
    let mut bindings = Vec::new();
    for project in &config.projects {
        let claude = Arc::new(ClaudeAdapter::new(&project.project_root));
        for path in &project.claude_sources {
            bindings.push(CaptureBinding::new(
                claude.clone(),
                SourceDescriptor::file(path),
                NormalizeContext {
                    project_id: project.project_id,
                    worktree_id: project.worktree_id,
                    source_schema: "claude-jsonl:auto".to_owned(),
                },
                &project.ledger_path,
            ));
        }

        let codex = Arc::new(CodexAdapter::new(project.codex_sources.clone()));
        for path in &project.codex_sources {
            bindings.push(CaptureBinding::new(
                codex.clone(),
                SourceDescriptor::file(path),
                NormalizeContext {
                    project_id: project.project_id,
                    worktree_id: project.worktree_id,
                    source_schema: "codex-rollout:auto".to_owned(),
                },
                &project.ledger_path,
            ));
        }

        if let Some(path) = &project.hermes_database {
            bindings.push(CaptureBinding::new(
                Arc::new(HermesAdapter::reviewed_for_project(
                    path,
                    &project.project_root,
                )?),
                SourceDescriptor::file(path),
                NormalizeContext {
                    project_id: project.project_id,
                    worktree_id: project.worktree_id,
                    source_schema: "hermes-state:v22".to_owned(),
                },
                &project.ledger_path,
            ));
        }
    }
    Ok(bindings)
}

pub fn build_hook_bindings(config: &ServiceLaunchConfig) -> Vec<HookProjectBinding> {
    config
        .projects
        .iter()
        .map(|project| HookProjectBinding {
            project_root: project.project_root.clone(),
            project_id: project.project_id,
            worktree_id: project.worktree_id,
            ledger_path: project.ledger_path.clone(),
        })
        .collect()
}
