use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_context::{
    ActivationDecision, CodeGraphActivationReport, CodeGraphClient, CodeGraphIndex,
    CodeGraphProvider, LlmWikiProvider, ProcessCodeGraphClient, ProviderConfig,
    codegraph_activation_decision,
};
use brain_coordination::CoordinationStore;
use brain_domain::{ProjectId, ProjectRegistry, WorktreeId};
use brain_service::{ServiceLaunchConfig, ServiceProjectConfig};
use brain_store::ProviderCacheStore;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Codegraph,
    LlmWiki,
}

impl ProviderKind {
    fn cache_name(self) -> &'static str {
        match self {
            Self::Codegraph => "codegraph",
            Self::LlmWiki => "llm_wiki",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProviderStatusReport {
    pub project_id: ProjectId,
    pub provider: ProviderKind,
    pub enabled: bool,
    pub usable: bool,
    pub detail: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProviderChangeReport {
    pub project_id: ProjectId,
    pub provider: ProviderKind,
    pub enabled: bool,
    pub removed_cache_entries: u64,
    pub external_data_preserved: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphIndexReport {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub index: CodeGraphIndex,
}

pub fn configure_llm_wiki(
    brain_home: &Path,
    project_selector: &str,
    vault: &Path,
) -> Result<ProviderChangeReport> {
    let project = resolve_project(brain_home, project_selector)?;
    let config_path = provider_config_path(&project)?;
    let mut config = ProviderConfig::load(&config_path, brain_home)?;
    config.llm_wiki.enabled = true;
    config.llm_wiki.vault = Some(vault.to_path_buf());
    config.validate(brain_home)?;
    // Construction additionally rejects missing, nested, and non-canonical vaults.
    LlmWikiProvider::new(
        project.project_id,
        project.worktree_id,
        vault,
        brain_home,
        Duration::from_millis(config.llm_wiki.deadline_ms),
        config.llm_wiki.max_results,
        config.llm_wiki.max_tokens,
    )?;
    save_provider_config(&config_path, &config)?;
    Ok(change_report(
        project.project_id,
        ProviderKind::LlmWiki,
        true,
        0,
    ))
}

pub fn configure_codegraph(
    brain_home: &Path,
    project_selector: &str,
    executable: &Path,
    activation_report_path: &Path,
) -> Result<ProviderChangeReport> {
    ensure!(executable.is_file(), "CodeGraph executable does not exist");
    let report_bytes = std::fs::read(activation_report_path).with_context(|| {
        format!(
            "read CodeGraph activation report {}",
            activation_report_path.display()
        )
    })?;
    let report: CodeGraphActivationReport = serde_json::from_slice(&report_bytes)?;
    ensure!(
        codegraph_activation_decision(&report) == ActivationDecision::Activate,
        "CodeGraph activation report did not pass every locked quality gate"
    );
    let client = ProcessCodeGraphClient::with_deadline(executable, Duration::from_secs(5))?;
    let capabilities = client.capabilities()?;
    ensure!(
        capabilities.schema_version == 1
            && capabilities.structured_search
            && capabilities.index_identity,
        "CodeGraph executable lacks the required structured interface"
    );
    ensure!(
        capabilities.provider_version == report.provider_version,
        "CodeGraph activation report belongs to a different provider version"
    );
    let project = resolve_project(brain_home, project_selector)?;
    let config_path = provider_config_path(&project)?;
    let mut config = ProviderConfig::load(&config_path, brain_home)?;
    config.codegraph.enabled = true;
    config.codegraph.executable = Some(executable.to_path_buf());
    config.codegraph.activation_report_sha256 = Some(Sha256::digest(report_bytes).into());
    config.validate(brain_home)?;
    save_provider_config(&config_path, &config)?;
    Ok(change_report(
        project.project_id,
        ProviderKind::Codegraph,
        true,
        0,
    ))
}

pub fn provider_status(
    brain_home: &Path,
    project_selector: &str,
) -> Result<Vec<ProviderStatusReport>> {
    let project = resolve_project(brain_home, project_selector)?;
    let config = ProviderConfig::load(provider_config_path(&project)?, brain_home)?;
    let llm_wiki = if !config.llm_wiki.enabled {
        status_report(
            project.project_id,
            ProviderKind::LlmWiki,
            false,
            false,
            "disabled by default",
        )
    } else if let Some(vault) = config.llm_wiki.vault.as_ref() {
        match LlmWikiProvider::new(
            project.project_id,
            project.worktree_id,
            vault,
            brain_home,
            Duration::from_millis(config.llm_wiki.deadline_ms),
            config.llm_wiki.max_results,
            config.llm_wiki.max_tokens,
        ) {
            Ok(provider) => status_report(
                project.project_id,
                ProviderKind::LlmWiki,
                true,
                true,
                format!(
                    "reviewed Markdown vault; source version {}",
                    provider.source_version()?
                ),
            ),
            Err(error) => status_report(
                project.project_id,
                ProviderKind::LlmWiki,
                true,
                false,
                error.to_string(),
            ),
        }
    } else {
        status_report(
            project.project_id,
            ProviderKind::LlmWiki,
            true,
            false,
            "vault is not configured",
        )
    };
    let codegraph = if !config.codegraph.enabled {
        status_report(
            project.project_id,
            ProviderKind::Codegraph,
            false,
            false,
            "disabled by default",
        )
    } else {
        match codegraph_provider(
            &project,
            &config,
            &project.project_root,
            project.worktree_id,
        ) {
            Ok(provider) => match provider.validate_status() {
                Ok(index) => status_report(
                    project.project_id,
                    ProviderKind::Codegraph,
                    true,
                    true,
                    format!("current index {} at {}", index.index_id, index.git_head),
                ),
                Err(error) => status_report(
                    project.project_id,
                    ProviderKind::Codegraph,
                    true,
                    false,
                    error.to_string(),
                ),
            },
            Err(error) => status_report(
                project.project_id,
                ProviderKind::Codegraph,
                true,
                false,
                error.to_string(),
            ),
        }
    };
    Ok(vec![codegraph, llm_wiki])
}

pub fn index_codegraph(
    brain_home: &Path,
    project_selector: &str,
    task_id: Option<uuid::Uuid>,
) -> Result<CodeGraphIndexReport> {
    let project = resolve_project(brain_home, project_selector)?;
    let config = ProviderConfig::load(provider_config_path(&project)?, brain_home)?;
    ensure!(config.codegraph.enabled, "CodeGraph is disabled");
    let (worktree_path, worktree_id) = if let Some(task_id) = task_id {
        let task = CoordinationStore::open(&project.ledger_path, project.project_id)?
            .task(task_id)?
            .context("CodeGraph task does not exist")?;
        (
            task.worktree_path
                .context("CodeGraph task has no validated worktree")?,
            task.worktree_id,
        )
    } else {
        (project.project_root.clone(), project.worktree_id)
    };
    let provider = codegraph_provider(&project, &config, &worktree_path, worktree_id)?;
    let index = provider.refresh_index()?;
    provider.validate_status()?;
    Ok(CodeGraphIndexReport {
        project_id: project.project_id,
        worktree_id,
        index,
    })
}

pub fn disable_provider(
    brain_home: &Path,
    project_selector: &str,
    provider: ProviderKind,
) -> Result<ProviderChangeReport> {
    let project = resolve_project(brain_home, project_selector)?;
    let config_path = provider_config_path(&project)?;
    let mut config = ProviderConfig::load(&config_path, brain_home)?;
    match provider {
        ProviderKind::Codegraph => config.codegraph.enabled = false,
        ProviderKind::LlmWiki => config.llm_wiki.enabled = false,
    }
    save_provider_config(&config_path, &config)?;
    Ok(change_report(project.project_id, provider, false, 0))
}

pub fn remove_provider(
    brain_home: &Path,
    project_selector: &str,
    provider: ProviderKind,
) -> Result<ProviderChangeReport> {
    let project = resolve_project(brain_home, project_selector)?;
    let config_path = provider_config_path(&project)?;
    let mut config = ProviderConfig::load(&config_path, brain_home)?;
    match provider {
        ProviderKind::Codegraph => config.codegraph = Default::default(),
        ProviderKind::LlmWiki => config.llm_wiki = Default::default(),
    }
    save_provider_config(&config_path, &config)?;
    let removed_cache_entries = ProviderCacheStore::open(&project.ledger_path, project.project_id)?
        .remove_provider_cache(provider.cache_name())?;
    Ok(change_report(
        project.project_id,
        provider,
        false,
        removed_cache_entries,
    ))
}

fn codegraph_provider(
    project: &ServiceProjectConfig,
    config: &ProviderConfig,
    worktree_path: &Path,
    worktree_id: WorktreeId,
) -> Result<CodeGraphProvider> {
    ensure!(
        config.codegraph.activation_report_sha256.is_some(),
        "CodeGraph activation benchmark has not passed"
    );
    let executable = config
        .codegraph
        .executable
        .as_ref()
        .context("CodeGraph executable is not configured")?;
    let client = Arc::new(ProcessCodeGraphClient::with_deadline(
        executable,
        Duration::from_millis(config.codegraph.deadline_ms),
    )?);
    CodeGraphProvider::new(
        client,
        project.project_id,
        worktree_id,
        worktree_path,
        git_head(worktree_path)?,
        true,
    )
}

fn git_head(worktree: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &worktree.to_string_lossy(), "rev-parse", "HEAD"])
        .output()?;
    ensure!(
        output.status.success(),
        "cannot resolve Git HEAD for {}",
        worktree.display()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn resolve_project(brain_home: &Path, project_selector: &str) -> Result<ServiceProjectConfig> {
    ensure!(!project_selector.trim().is_empty(), "project is required");
    let project_id = ProjectRegistry::open(brain_home)?.resolve(project_selector)?;
    Ok(
        ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(brain_home))?
            .project(Some(project_id))?
            .clone(),
    )
}

fn provider_config_path(project: &ServiceProjectConfig) -> Result<PathBuf> {
    Ok(project
        .ledger_path
        .parent()
        .context("project ledger has no storage directory")?
        .join("providers.json"))
}

fn save_provider_config(path: &Path, config: &ProviderConfig) -> Result<()> {
    let parent = path.parent().context("provider config has no parent")?;
    std::fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(config)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .with_context(|| format!("atomically write provider config {}", path.display()))
}

fn status_report(
    project_id: ProjectId,
    provider: ProviderKind,
    enabled: bool,
    usable: bool,
    detail: impl Into<String>,
) -> ProviderStatusReport {
    ProviderStatusReport {
        project_id,
        provider,
        enabled,
        usable,
        detail: detail.into(),
    }
}

fn change_report(
    project_id: ProjectId,
    provider: ProviderKind,
    enabled: bool,
    removed_cache_entries: u64,
) -> ProviderChangeReport {
    ProviderChangeReport {
        project_id,
        provider,
        enabled,
        removed_cache_entries,
        external_data_preserved: true,
    }
}
