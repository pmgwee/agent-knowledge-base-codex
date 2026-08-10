use std::collections::HashSet;
use std::time::Duration;
use std::{path::Path, path::PathBuf};

use anyhow::{Context, Result, bail};
use brain_domain::{ProjectId, WorktreeId};

const SERVICE_CONFIG_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureServiceConfig {
    pub reconciliation_interval: Duration,
    pub watcher_debounce: Duration,
}

impl CaptureServiceConfig {
    pub fn validate(&self) -> Result<()> {
        if self.reconciliation_interval.is_zero() {
            bail!("reconciliation interval must be greater than zero");
        }
        if self.watcher_debounce.is_zero() {
            bail!("watcher debounce must be greater than zero");
        }
        Ok(())
    }
}

impl Default for CaptureServiceConfig {
    fn default() -> Self {
        Self {
            reconciliation_interval: Duration::from_secs(2),
            watcher_debounce: Duration::from_millis(50),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceLaunchConfig {
    #[serde(default = "service_config_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_pipe_name")]
    pub pipe_name: String,
    #[serde(default)]
    pub consolidation: Option<ConsolidationProviderConfig>,
    #[serde(default)]
    pub review: ReviewGateConfig,
    pub projects: Vec<ServiceProjectConfig>,
}

/// Which consolidated memory kinds a person must approve before anything reads them.
///
/// **A9's ingest half.** A memory written `Proposed` fails `CURRENT_CLAIM`, so it is invisible to
/// the orientation, to `search`, and to the markdown projection until somebody approves it. The
/// mechanism already existed in the model and nothing had ever written it.
///
/// **Empty by default, and that default is a position rather than caution.** Gating a kind means
/// the brain stops telling agents about it until a human gets to it, and a review queue nobody
/// drains is a brain that forgets on purpose. `decision` is the kind worth the friction — 22% of
/// memories, the ones an orientation leans on hardest, and the ones whose falsehood is most
/// expensive — but turning it on is a commitment to actually reviewing, so it is the operator's
/// call and not a default.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct ReviewGateConfig {
    /// Memory kinds held at `Proposed` until reviewed, by their serialized name (`"decision"`).
    #[serde(default)]
    pub gated_kinds: Vec<String>,
}

impl ReviewGateConfig {
    pub fn gates(&self, kind: &brain_domain::MemoryKind) -> bool {
        let name = kind.as_str();
        self.gated_kinds
            .iter()
            .any(|gated| gated.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ConsolidationProviderConfig {
    Glm {
        endpoint: String,
        model: String,
        #[serde(default = "default_glm_key_env")]
        api_key_env: String,
        #[serde(default = "default_glm_timeout_ms")]
        timeout_ms: u64,
        #[serde(default = "default_glm_max_retries")]
        max_retries: u32,
    },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ServiceProjectConfig {
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
    #[serde(default)]
    pub claude_sources: Vec<PathBuf>,
    #[serde(default)]
    pub codex_sources: Vec<PathBuf>,
    #[serde(default)]
    pub hermes_database: Option<PathBuf>,
}

impl ServiceLaunchConfig {
    pub fn new(pipe_name: impl Into<String>) -> Self {
        Self {
            schema_version: SERVICE_CONFIG_SCHEMA_VERSION,
            pipe_name: pipe_name.into(),
            consolidation: None,
            review: ReviewGateConfig::default(),
            projects: Vec::new(),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref()).with_context(|| {
            format!(
                "read service launch configuration {}",
                path.as_ref().display()
            )
        })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "parse service launch configuration {}",
                path.as_ref().display()
            )
        })?;
        let config = if value.get("projects").is_some() {
            serde_json::from_value(value)?
        } else {
            let legacy: LegacyServiceLaunchConfig = serde_json::from_value(value)?;
            Self {
                schema_version: SERVICE_CONFIG_SCHEMA_VERSION,
                pipe_name: legacy.pipe_name,
                consolidation: None,
                review: ReviewGateConfig::default(),
                projects: vec![ServiceProjectConfig {
                    project_root: legacy.project_root,
                    project_id: legacy.project_id,
                    worktree_id: legacy.worktree_id,
                    ledger_path: legacy.ledger_path,
                    claude_sources: legacy.claude_sources,
                    codex_sources: Vec::new(),
                    hermes_database: None,
                }],
            }
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SERVICE_CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported service configuration schema {}; expected {}",
                self.schema_version,
                SERVICE_CONFIG_SCHEMA_VERSION
            );
        }
        let mut projects = HashSet::new();
        for project in &self.projects {
            if !projects.insert(project.project_id) {
                bail!(
                    "duplicate project {} in service configuration",
                    project.project_id.0
                );
            }
        }
        Ok(())
    }

    pub fn project(&self, project: Option<ProjectId>) -> Result<&ServiceProjectConfig> {
        match project {
            Some(project_id) => self
                .projects
                .iter()
                .find(|candidate| candidate.project_id == project_id)
                .with_context(|| format!("project {} is not configured", project_id.0)),
            None if self.projects.len() == 1 => Ok(&self.projects[0]),
            None if self.projects.is_empty() => bail!("no projects are configured"),
            None => bail!("--project is required when multiple projects are configured"),
        }
    }

    pub fn upsert_project(&mut self, project: ServiceProjectConfig) {
        if let Some(existing) = self
            .projects
            .iter_mut()
            .find(|candidate| candidate.project_id == project.project_id)
        {
            *existing = project;
        } else {
            self.projects.push(project);
        }
        self.projects
            .sort_by_key(|project| project.project_id.0.to_string());
    }

    pub fn default_path(brain_home: impl AsRef<Path>) -> PathBuf {
        brain_home.as_ref().join("runtime").join("service.json")
    }
}

#[derive(serde::Deserialize)]
struct LegacyServiceLaunchConfig {
    #[serde(default = "default_pipe_name")]
    pipe_name: String,
    project_root: PathBuf,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    ledger_path: PathBuf,
    #[serde(default)]
    claude_sources: Vec<PathBuf>,
}

const fn service_config_schema_version() -> u32 {
    SERVICE_CONFIG_SCHEMA_VERSION
}

fn default_pipe_name() -> String {
    r"\\.\pipe\agent-brain-v1".to_owned()
}

fn default_glm_key_env() -> String {
    "GLM_API_KEY".to_owned()
}

const fn default_glm_timeout_ms() -> u64 {
    30_000
}

const fn default_glm_max_retries() -> u32 {
    2
}
