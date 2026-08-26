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
    #[serde(default, deserialize_with = "consolidation_or_none")]
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

/// The consolidation provider, described in provider-neutral terms.
///
/// One variant, deliberately. Which vendor answers is decided by `base_url` and `model`, not by a
/// Rust variant per vendor — so moving to a different OpenAI-compatible endpoint is a
/// configuration edit rather than a change to this crate and everything that matches on it.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ConsolidationProviderConfig {
    Llm {
        /// The API **root**. The client appends `/responses`; do not include it here.
        #[serde(default = "default_llm_base_url")]
        base_url: String,
        #[serde(default = "default_llm_model")]
        model: String,
        /// The *name* of the environment variable holding the key, never the key.
        #[serde(default = "default_llm_key_env")]
        api_key_env: String,
        #[serde(default = "default_llm_timeout_ms")]
        timeout_ms: u64,
        #[serde(default = "default_llm_max_retries")]
        max_retries: u32,
    },
}

impl ConsolidationProviderConfig {
    /// The one place base URL, model and key-variable name are resolved.
    ///
    /// Environment wins over the file. `service.json` also holds the project list, so a
    /// deployment that only needs to point at a different endpoint or model should not have to
    /// rewrite it — and a deployment platform hands its configuration over as environment, not as
    /// a file it can edit.
    ///
    /// Errors name the setting that is missing and never its value: the key itself is not read
    /// here at all, only the name of the variable that holds it.
    pub fn resolve(&self) -> Result<brain_context::LlmConfig> {
        let Self::Llm {
            base_url,
            model,
            api_key_env,
            timeout_ms,
            max_retries,
        } = self;
        let base_url = environment_override("LLM_BASE_URL").unwrap_or_else(|| base_url.clone());
        let model = environment_override("LLM_MODEL").unwrap_or_else(|| model.clone());
        if base_url.trim().is_empty() {
            bail!(
                "no LLM base URL is configured; set LLM_BASE_URL or consolidation.base_url in the service configuration"
            );
        }
        if model.trim().is_empty() {
            bail!(
                "no LLM model is configured; set LLM_MODEL or consolidation.model in the service configuration"
            );
        }
        if api_key_env.trim().is_empty() {
            bail!(
                "no LLM key variable is configured; set consolidation.api_key_env in the service configuration (default {})",
                default_llm_key_env()
            );
        }
        if *timeout_ms == 0 {
            bail!("consolidation.timeout_ms must be greater than zero");
        }
        Ok(brain_context::LlmConfig {
            base_url,
            model,
            api_key_env: api_key_env.clone(),
            timeout: Duration::from_millis(*timeout_ms),
            max_retries: *max_retries,
        })
    }

    /// Build the adapter every consolidation path talks to.
    ///
    /// The service, `brain revise` and `brain synthesize` all construct it here rather than each
    /// assembling the same five fields — three copies of that construction is three places for a
    /// provider change to be applied twice and missed once.
    pub fn client(&self) -> Result<brain_context::LlmClient> {
        brain_context::LlmClient::new(self.resolve()?)
    }

    /// The model that will answer, after environment overrides. For display only.
    pub fn resolved_model(&self) -> String {
        let Self::Llm { model, .. } = self;
        environment_override("LLM_MODEL").unwrap_or_else(|| model.clone())
    }

    pub fn api_key_env(&self) -> &str {
        let Self::Llm { api_key_env, .. } = self;
        api_key_env
    }
}

/// An environment variable counts as set only when it holds something.
///
/// An empty variable is what a deployment platform leaves behind when a value is cleared, and
/// treating it as an override would replace a working configured value with nothing.
fn environment_override(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// A consolidation block this build does not understand disables consolidation; it does not stop
/// the service.
///
/// Consolidation is optional and `None` is a supported, tested state — capture, canonical
/// retrieval and the startup orientation all work without it. A stale block, such as one naming
/// the provider this system used before the move to a neutral adapter, would otherwise fail
/// `ServiceLaunchConfig::load`, and *every* caller of `load` fails with it: the service, every CLI
/// command, and the dashboard that shells out to them. Losing curated memory until the file is
/// corrected is a far smaller failure than losing capture, and the warning says which it is.
fn consolidation_or_none<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<ConsolidationProviderConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let Some(value) =
        Option::<serde_json::Value>::deserialize(deserializer)?.filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    match serde_json::from_value(value.clone()) {
        Ok(config) => Ok(Some(config)),
        Err(error) => {
            tracing::warn!(
                %error,
                provider = value
                    .get("provider")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unnamed"),
                "consolidation is configured with a block this build does not understand, so                  consolidation is disabled until it is corrected; capture and retrieval are                  unaffected"
            );
            Ok(None)
        }
    }
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

fn default_llm_key_env() -> String {
    "LLM_API_KEY".to_owned()
}

/// The preferred API root; the client also accepts a complete `/responses` endpoint once.
fn default_llm_base_url() -> String {
    "https://opencode.ai/zen/go/v1".to_owned()
}

fn default_llm_model() -> String {
    "gpt-5.6-luna".to_owned()
}

const fn default_llm_timeout_ms() -> u64 {
    30_000
}

const fn default_llm_max_retries() -> u32 {
    2
}
