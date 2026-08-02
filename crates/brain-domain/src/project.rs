use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::{ProjectId, WorktreeId};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectIdentity {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub project_key: String,
    pub worktree_key: String,
    pub root: PathBuf,
    pub git_common_dir: Option<PathBuf>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

impl ProjectIdentity {
    pub fn for_non_git(root: impl AsRef<Path>) -> Result<Self> {
        let canonical = canonical_path(root.as_ref())?;
        let key = path_key("path", &canonical);

        Ok(Self {
            project_id: ProjectId::unregistered(),
            worktree_id: WorktreeId::unregistered(),
            project_key: key.clone(),
            worktree_key: key,
            root: canonical,
            git_common_dir: None,
            branch: None,
            head: None,
        })
    }

    pub fn inspect(root: impl AsRef<Path>) -> Result<Self> {
        let canonical_root = canonical_path(root.as_ref())?;
        let Some(common_dir_text) = git_output(&canonical_root, &["rev-parse", "--git-common-dir"])
        else {
            return Self::for_non_git(canonical_root);
        };
        let common_dir_path = PathBuf::from(common_dir_text);
        let common_dir = if common_dir_path.is_absolute() {
            canonical_path(&common_dir_path)?
        } else {
            canonical_path(&canonical_root.join(common_dir_path))?
        };

        Ok(Self {
            project_id: ProjectId::unregistered(),
            worktree_id: WorktreeId::unregistered(),
            project_key: path_key("git-common-dir", &common_dir),
            worktree_key: path_key("worktree", &canonical_root),
            root: canonical_root.clone(),
            git_common_dir: Some(common_dir),
            branch: git_output(&canonical_root, &["branch", "--show-current"])
                .filter(|value| !value.is_empty()),
            head: git_output(&canonical_root, &["rev-parse", "HEAD"]),
        })
    }
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("failed to resolve project root {}", path.display()))
}

fn path_key(prefix: &str, path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{prefix}:{digest:x}")
}

fn git_output(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}

pub(crate) fn normalized_path(path: &Path) -> Result<String> {
    Ok(canonical_path(path)?
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase())
}

pub(crate) fn remote_url(root: &Path) -> Option<String> {
    git_output(root, &["remote", "get-url", "origin"])
}
