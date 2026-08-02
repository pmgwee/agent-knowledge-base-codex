use std::time::Duration;
use std::{path::Path, path::PathBuf};

use anyhow::{Context, Result, bail};
use brain_domain::{ProjectId, WorktreeId};

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
    #[serde(default = "default_pipe_name")]
    pub pipe_name: String,
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
    #[serde(default)]
    pub claude_sources: Vec<PathBuf>,
}

impl ServiceLaunchConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref()).with_context(|| {
            format!(
                "read service launch configuration {}",
                path.as_ref().display()
            )
        })?;
        serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "parse service launch configuration {}",
                path.as_ref().display()
            )
        })
    }

    pub fn default_path(brain_home: impl AsRef<Path>) -> PathBuf {
        brain_home.as_ref().join("runtime").join("service.json")
    }
}

fn default_pipe_name() -> String {
    r"\\.\pipe\agent-brain-v1".to_owned()
}
