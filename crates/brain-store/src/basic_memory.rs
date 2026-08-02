use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use crate::ProjectionReport;

pub const BASIC_MEMORY_PINNED_VERSION: &str = "0.22.1";

pub trait BasicMemoryCli: Send + Sync {
    fn version(&self) -> Result<String>;
    fn ensure_project(&self, name: &str, root: &Path) -> Result<()>;
    fn reindex(&self, name: &str) -> Result<()>;
}

pub struct ProcessBasicMemoryCli {
    executable: PathBuf,
}

impl ProcessBasicMemoryCli {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl Default for ProcessBasicMemoryCli {
    fn default() -> Self {
        Self::new("basic-memory")
    }
}

impl BasicMemoryCli for ProcessBasicMemoryCli {
    fn version(&self) -> Result<String> {
        let output = self.run(["--version"])?;
        successful_text(output, "read Basic Memory version")
    }

    fn ensure_project(&self, name: &str, root: &Path) -> Result<()> {
        let root = root
            .to_str()
            .context("Basic Memory project path is not valid Unicode")?;
        let output = self.run(["project", "add", name, root])?;
        if output.status.success() {
            return Ok(());
        }
        let diagnostic = bounded_diagnostic(&output);
        if diagnostic.to_ascii_lowercase().contains("already exists") {
            return Ok(());
        }
        bail!("Basic Memory project registration failed: {diagnostic}")
    }

    fn reindex(&self, name: &str) -> Result<()> {
        let output = self.run(["--project", name, "reindex"])?;
        successful_text(output, "reindex Basic Memory project").map(|_| ())
    }
}

impl ProcessBasicMemoryCli {
    fn run<const N: usize>(&self, arguments: [&str; N]) -> Result<Output> {
        Command::new(&self.executable)
            .args(arguments)
            .env("BASIC_MEMORY_NO_PROMOS", "1")
            .output()
            .with_context(|| {
                format!(
                    "Basic Memory executable {} is unavailable",
                    self.executable.display()
                )
            })
    }
}

pub struct BasicMemoryIndexer<'a> {
    cli: &'a dyn BasicMemoryCli,
}

impl<'a> BasicMemoryIndexer<'a> {
    pub fn new(cli: &'a dyn BasicMemoryCli) -> Self {
        Self { cli }
    }

    pub fn rebuild(&self, projection: &ProjectionReport) -> BasicMemoryReport {
        let project_name = format!("agent-brain-{}", projection.project_id.0.simple());
        let index_root = basic_memory_view_root(projection);
        let result = (|| -> Result<()> {
            let version = self.cli.version()?;
            ensure!(
                version
                    .split_whitespace()
                    .any(|part| part == BASIC_MEMORY_PINNED_VERSION),
                "Basic Memory version mismatch: expected {BASIC_MEMORY_PINNED_VERSION}"
            );
            materialize_view(projection, &index_root)?;
            self.cli.ensure_project(&project_name, &index_root)?;
            self.cli.reindex(&project_name)?;
            Ok(())
        })();
        match result {
            Ok(()) => BasicMemoryReport {
                state: BasicMemoryState::Ready,
                project_name,
                index_root,
                projected_files: projection.file_count,
                reason: None,
            },
            Err(error) => BasicMemoryReport {
                state: BasicMemoryState::Degraded,
                project_name,
                index_root,
                projected_files: projection.file_count,
                reason: Some(error.to_string()),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BasicMemoryState {
    Ready,
    Degraded,
}

#[derive(Clone, Debug, Serialize)]
pub struct BasicMemoryReport {
    pub state: BasicMemoryState,
    pub project_name: String,
    pub index_root: PathBuf,
    pub projected_files: usize,
    pub reason: Option<String>,
}

fn basic_memory_view_root(projection: &ProjectionReport) -> PathBuf {
    projection
        .manifest_path
        .parent()
        .and_then(Path::parent)
        .expect("projection manifest lives below a project root")
        .join("basic-memory")
}

fn materialize_view(projection: &ProjectionReport, index_root: &Path) -> Result<()> {
    let project_root = index_root
        .parent()
        .context("Basic Memory view must have a project parent")?;
    std::fs::create_dir_all(project_root)?;
    let staging = project_root.join(format!("basic-memory-staging-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir(&staging)?;
    copy_tree(&projection.generation_root, &staging)?;
    let backup = project_root.join(format!("basic-memory-backup-{}", uuid::Uuid::now_v7()));
    if index_root.exists() {
        std::fs::rename(index_root, &backup)?;
    }
    if let Err(error) = std::fs::rename(&staging, index_root) {
        if backup.exists() && !index_root.exists() {
            let _ = std::fs::rename(&backup, index_root);
        }
        return Err(error).context("publish Basic Memory disposable view");
    }
    if backup.exists() {
        remove_owned_temporary(&backup, project_root, "basic-memory-backup-")?;
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let mut directories = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source, destination)) = directories.pop() {
        for entry in std::fs::read_dir(&source)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            ensure!(
                !file_type.is_symlink(),
                "generated projection contains a symlink"
            );
            let target = destination.join(entry.file_name());
            if file_type.is_dir() {
                std::fs::create_dir(&target)?;
                directories.push((entry.path(), target));
            } else if file_type.is_file() {
                std::fs::copy(entry.path(), target)?;
            }
        }
    }
    Ok(())
}

fn remove_owned_temporary(path: &Path, parent: &Path, prefix: &str) -> Result<()> {
    ensure!(
        path.parent() == Some(parent),
        "refusing to remove a view outside its project root"
    );
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("temporary view name is invalid")?;
    ensure!(
        name.starts_with(prefix),
        "refusing to remove an unowned view directory"
    );
    std::fs::remove_dir_all(path)?;
    Ok(())
}

fn successful_text(output: Output, action: &str) -> Result<String> {
    if !output.status.success() {
        bail!("{action} failed: {}", bounded_diagnostic(&output));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn bounded_diagnostic(output: &Output) -> String {
    let value = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    String::from_utf8_lossy(value)
        .chars()
        .take(500)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
