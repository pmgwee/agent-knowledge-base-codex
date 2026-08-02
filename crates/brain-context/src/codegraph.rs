use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use brain_domain::{ProjectId, WorktreeId};

use crate::{ContextProvider, ContextQuery, ProviderResult};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphCapabilities {
    pub schema_version: u32,
    pub provider_version: String,
    pub structured_search: bool,
    pub index_identity: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphIndex {
    pub index_id: String,
    pub worktree_path: PathBuf,
    pub git_head: String,
    pub provider_version: String,
    pub indexed_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphHit {
    pub file: PathBuf,
    pub symbol: Option<String>,
    pub line_start: Option<u64>,
    pub line_end: Option<u64>,
    pub relationship: Option<String>,
    pub excerpt: String,
    pub score: f64,
}

pub trait CodeGraphClient: Send + Sync {
    fn capabilities(&self) -> Result<CodeGraphCapabilities>;
    fn index(&self, worktree: &Path) -> Result<CodeGraphIndex>;
    fn status(&self, worktree: &Path) -> Result<CodeGraphIndex>;
    fn search(&self, worktree: &Path, query: &str, limit: usize) -> Result<Vec<CodeGraphHit>>;
}

pub struct ProcessCodeGraphClient {
    executable: PathBuf,
    deadline: std::time::Duration,
}

impl ProcessCodeGraphClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            deadline: std::time::Duration::from_secs(5),
        }
    }

    pub fn with_deadline(
        executable: impl Into<PathBuf>,
        deadline: std::time::Duration,
    ) -> Result<Self> {
        ensure!(
            deadline > std::time::Duration::ZERO,
            "CodeGraph deadline is zero"
        );
        Ok(Self {
            executable: executable.into(),
            deadline,
        })
    }

    fn json<T: serde::de::DeserializeOwned>(&self, arguments: &[&str]) -> Result<T> {
        let mut child = Command::new(&self.executable)
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("execute CodeGraph {}", self.executable.display()))?;
        let started = std::time::Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if started.elapsed() >= self.deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "CodeGraph command exceeded {} ms",
                    self.deadline.as_millis()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "CodeGraph command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        serde_json::from_slice(&output.stdout).context("parse CodeGraph structured JSON")
    }
}

impl CodeGraphClient for ProcessCodeGraphClient {
    fn capabilities(&self) -> Result<CodeGraphCapabilities> {
        self.json(&["capabilities", "--json"])
    }

    fn index(&self, worktree: &Path) -> Result<CodeGraphIndex> {
        self.json(&[
            "index",
            "--root",
            external_path(worktree).as_str(),
            "--json",
        ])
    }

    fn status(&self, worktree: &Path) -> Result<CodeGraphIndex> {
        self.json(&[
            "status",
            "--root",
            external_path(worktree).as_str(),
            "--json",
        ])
    }

    fn search(&self, worktree: &Path, query: &str, limit: usize) -> Result<Vec<CodeGraphHit>> {
        #[derive(serde::Deserialize)]
        struct Response {
            hits: Vec<CodeGraphHit>,
        }
        let limit = limit.min(20).to_string();
        Ok(self
            .json::<Response>(&[
                "search",
                "--root",
                external_path(worktree).as_str(),
                "--query",
                query,
                "--limit",
                &limit,
                "--json",
            ])?
            .hits)
    }
}

pub struct CodeGraphProvider {
    client: Arc<dyn CodeGraphClient>,
    project_id: ProjectId,
    worktree_id: WorktreeId,
    worktree_path: PathBuf,
    git_head: String,
    activated: bool,
}

impl CodeGraphProvider {
    pub fn new(
        client: Arc<dyn CodeGraphClient>,
        project_id: ProjectId,
        worktree_id: WorktreeId,
        worktree_path: impl AsRef<Path>,
        git_head: impl Into<String>,
        activated: bool,
    ) -> Result<Self> {
        let worktree_path = std::fs::canonicalize(worktree_path.as_ref())?;
        Ok(Self {
            client,
            project_id,
            worktree_id,
            worktree_path,
            git_head: git_head.into(),
            activated,
        })
    }

    pub fn validate_status(&self) -> Result<CodeGraphIndex> {
        ensure!(self.activated, "CodeGraph activation gate has not passed");
        let capabilities = self.client.capabilities()?;
        ensure!(
            capabilities.schema_version == 1
                && capabilities.structured_search
                && capabilities.index_identity,
            "CodeGraph lacks structured search or reliable index identity"
        );
        let status = self.client.status(&self.worktree_path)?;
        ensure!(
            same_path(&status.worktree_path, &self.worktree_path),
            "CodeGraph index belongs to another worktree"
        );
        ensure!(
            status.git_head == self.git_head,
            "CodeGraph index is stale for current HEAD"
        );
        ensure!(
            status.provider_version == capabilities.provider_version,
            "CodeGraph index provider version changed"
        );
        Ok(status)
    }

    pub fn refresh_index(&self) -> Result<CodeGraphIndex> {
        let index = self.client.index(&self.worktree_path)?;
        ensure!(
            same_path(&index.worktree_path, &self.worktree_path),
            "CodeGraph indexed another worktree"
        );
        ensure!(
            index.git_head == self.git_head,
            "CodeGraph indexed a different HEAD"
        );
        Ok(index)
    }

    pub fn search_blocking(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        ensure!(
            query.project_id == self.project_id,
            "CodeGraph project scope mismatch"
        );
        ensure!(
            query.worktree_id == self.worktree_id,
            "CodeGraph worktree scope mismatch"
        );
        let prompt = query.prompt.as_deref().unwrap_or("").trim();
        if prompt.is_empty() && query.paths.is_empty() {
            return Ok(Vec::new());
        }
        let prompt = if prompt.is_empty() {
            query.paths.join(" ")
        } else {
            prompt.to_owned()
        };
        let status = validate_client(
            self.client.as_ref(),
            &self.worktree_path,
            &self.git_head,
            self.activated,
        )?;
        let hits = self.client.search(&self.worktree_path, &prompt, 8)?;
        let now = time::OffsetDateTime::now_utc();
        Ok(hits
            .into_iter()
            .filter(|hit| hit.score.is_finite() && (0.0..=1.0).contains(&hit.score))
            .filter_map(|hit| {
                let file = checked_file(&self.worktree_path, &hit.file)?;
                Some(ProviderResult {
                    provider: "codegraph".to_owned(),
                    project_id: self.project_id,
                    worktree_id: Some(self.worktree_id),
                    title: hit
                        .symbol
                        .clone()
                        .unwrap_or_else(|| file.to_string_lossy().to_string()),
                    content: format!(
                        "{}{}",
                        hit.excerpt,
                        hit.relationship
                            .as_deref()
                            .map(|value| format!("; relationship={value}"))
                            .unwrap_or_default()
                    ),
                    source_uri: format!(
                        "{}#L{}-L{}",
                        file.to_string_lossy().replace('\\', "/"),
                        hit.line_start.unwrap_or(1),
                        hit.line_end.or(hit.line_start).unwrap_or(1)
                    ),
                    source_date: Some(status.indexed_at),
                    observed_at: now,
                    trust: "current_code_index".to_owned(),
                    relevance: hit.score,
                    git_head: Some(self.git_head.clone()),
                })
            })
            .collect())
    }
}

#[async_trait]
impl ContextProvider for CodeGraphProvider {
    async fn retrieve(&self, query: &ContextQuery) -> Result<Vec<ProviderResult>> {
        let provider = Self {
            client: Arc::clone(&self.client),
            project_id: self.project_id,
            worktree_id: self.worktree_id,
            worktree_path: self.worktree_path.clone(),
            git_head: self.git_head.clone(),
            activated: self.activated,
        };
        let query = query.clone();
        tokio::task::spawn_blocking(move || provider.search_blocking(&query)).await?
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CodeGraphActivationReport {
    pub provider_version: String,
    pub repository_size_class: String,
    pub baseline_targeted_read_tokens: u64,
    pub provider_targeted_read_tokens: u64,
    pub baseline_accuracy: f64,
    pub provider_accuracy: f64,
    pub all_citations_current: bool,
    pub cold_index_seconds: f64,
    pub average_session_seconds_saved: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationDecision {
    Activate,
    KeepDisabled,
}

pub fn codegraph_activation_decision(report: &CodeGraphActivationReport) -> ActivationDecision {
    let token_reduction = if report.baseline_targeted_read_tokens == 0 {
        0.0
    } else {
        1.0 - report.provider_targeted_read_tokens as f64
            / report.baseline_targeted_read_tokens as f64
    };
    let amortized_sessions = if report.average_session_seconds_saved <= 0.0 {
        f64::INFINITY
    } else {
        report.cold_index_seconds / report.average_session_seconds_saved
    };
    if token_reduction >= 0.20
        && report.provider_accuracy >= report.baseline_accuracy
        && report.all_citations_current
        && amortized_sessions <= 20.0
    {
        ActivationDecision::Activate
    } else {
        ActivationDecision::KeepDisabled
    }
}

fn checked_file(root: &Path, file: &Path) -> Option<PathBuf> {
    let candidate = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let candidate = std::fs::canonicalize(candidate).ok()?;
    candidate.starts_with(root).then_some(candidate)
}

fn validate_client(
    client: &dyn CodeGraphClient,
    worktree: &Path,
    head: &str,
    activated: bool,
) -> Result<CodeGraphIndex> {
    ensure!(activated, "CodeGraph activation gate has not passed");
    let capabilities = client.capabilities()?;
    ensure!(
        capabilities.schema_version == 1
            && capabilities.structured_search
            && capabilities.index_identity,
        "CodeGraph lacks structured search or reliable index identity"
    );
    let status = client.status(worktree)?;
    ensure!(
        same_path(&status.worktree_path, worktree),
        "CodeGraph index belongs to another worktree"
    );
    ensure!(
        status.git_head == head,
        "CodeGraph index is stale for current HEAD"
    );
    ensure!(
        status.provider_version == capabilities.provider_version,
        "CodeGraph index provider version changed"
    );
    Ok(status)
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left.to_string_lossy().replace('/', "\\").to_lowercase()
        == right.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn external_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}
