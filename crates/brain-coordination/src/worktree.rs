use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use brain_domain::{ProjectId, ProjectRegistry};

use crate::{CoordinationStore, TaskRecord, TaskStatus};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CloseTaskReport {
    pub task: TaskRecord,
    pub dirty: bool,
    pub ahead_of_upstream: Option<u64>,
    pub cleanup_command: Option<String>,
}

pub struct TaskWorktreeManager {
    brain_home: PathBuf,
    project_id: ProjectId,
    project_root: PathBuf,
    ledger_path: PathBuf,
    worktree_parent: PathBuf,
}

impl TaskWorktreeManager {
    pub fn new(
        brain_home: impl AsRef<Path>,
        project_id: ProjectId,
        project_root: impl AsRef<Path>,
        ledger_path: impl AsRef<Path>,
        worktree_parent: impl AsRef<Path>,
    ) -> Result<Self> {
        let project_root = std::fs::canonicalize(project_root.as_ref())
            .with_context(|| format!("resolve project root {}", project_root.as_ref().display()))?;
        let registry = ProjectRegistry::open(brain_home.as_ref())?;
        ensure!(
            registry.resolve(project_root.to_string_lossy().as_ref())? == project_id,
            "project root does not match the selected project"
        );
        ensure!(
            git_output(&project_root, &["rev-parse", "--git-common-dir"]).is_ok(),
            "task worktrees require a Git repository"
        );
        std::fs::create_dir_all(worktree_parent.as_ref())?;
        let worktree_parent =
            std::fs::canonicalize(worktree_parent.as_ref()).with_context(|| {
                format!(
                    "resolve approved worktree parent {}",
                    worktree_parent.as_ref().display()
                )
            })?;
        Ok(Self {
            brain_home: brain_home.as_ref().to_path_buf(),
            project_id,
            project_root,
            ledger_path: ledger_path.as_ref().to_path_buf(),
            worktree_parent,
        })
    }

    pub fn create(
        &self,
        title: &str,
        base: Option<&str>,
        now: time::OffsetDateTime,
    ) -> Result<TaskRecord> {
        ensure!(!title.trim().is_empty(), "task title is required");
        let task_id = uuid::Uuid::now_v7();
        let suffix = &task_id.simple().to_string()[..8];
        let slug = slug(title);
        let branch = format!("agent/{slug}-{suffix}");
        let path = self.worktree_parent.join(format!("{slug}-{suffix}"));
        ensure!(
            path.starts_with(&self.worktree_parent),
            "worktree path escaped approved parent"
        );
        ensure!(
            !path.exists(),
            "worktree path already exists: {}",
            path.display()
        );
        let base = base.unwrap_or("HEAD");
        git_output(
            &self.project_root,
            &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
        )
        .with_context(|| format!("base ref {base:?} is not a commit"))?;
        let git_path = external_path(&path);
        git_success(
            &self.project_root,
            &["worktree", "add", "-b", &branch, &git_path, base],
        )?;
        let identity =
            match ProjectRegistry::open(&self.brain_home)?.add_alias(self.project_id, &path) {
                Ok(identity) => identity,
                Err(error) => {
                    let _ = git_success(&self.project_root, &["worktree", "remove", &git_path]);
                    return Err(error);
                }
            };
        let task = TaskRecord {
            id: task_id,
            project_id: self.project_id,
            worktree_id: identity.worktree_id,
            title: title.trim().to_owned(),
            worktree_path: Some(identity.root),
            branch: Some(branch),
            status: TaskStatus::Active,
            created_at: now,
            closed_at: None,
        };
        let mut store = CoordinationStore::open(&self.ledger_path, self.project_id)?;
        if let Err(error) = store.create_task(&task) {
            let _ = git_success(&self.project_root, &["worktree", "remove", &git_path]);
            return Err(error);
        }
        Ok(task)
    }

    pub fn list_git_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let output = git_output(&self.project_root, &["worktree", "list", "--porcelain"])?;
        let mut worktrees = parse_worktrees(&output)?;
        for worktree in &mut worktrees {
            if worktree.path.exists() {
                worktree.path = std::fs::canonicalize(&worktree.path)?;
            }
        }
        Ok(worktrees)
    }

    pub fn tasks(&self, include_closed: bool) -> Result<Vec<TaskRecord>> {
        CoordinationStore::open(&self.ledger_path, self.project_id)?.tasks(include_closed)
    }

    pub fn close(&self, task_id: uuid::Uuid, now: time::OffsetDateTime) -> Result<CloseTaskReport> {
        let mut store = CoordinationStore::open(&self.ledger_path, self.project_id)?;
        let task = store.task(task_id)?.context("task does not exist")?;
        if let Some(lease) = store.lease(task_id)? {
            store
                .release_lease(task_id, &lease.owner, lease.generation, now)
                .map_err(anyhow::Error::from)?;
        }
        let path = task.worktree_path.as_deref();
        let dirty = path
            .map(|path| git_output(path, &["status", "--porcelain"]))
            .transpose()?
            .is_some_and(|output| !output.trim().is_empty());
        let ahead_of_upstream = path.and_then(|path| {
            git_output(path, &["rev-list", "--count", "@{upstream}..HEAD"])
                .ok()?
                .trim()
                .parse()
                .ok()
        });
        let task = store.close_task(task_id, TaskStatus::Completed, now)?;
        let cleanup_command = path.map(|path| {
            format!(
                "git -C {} worktree remove {}",
                quote(&self.project_root),
                quote(path)
            )
        });
        Ok(CloseTaskReport {
            task,
            dirty,
            ahead_of_upstream,
            cleanup_command,
        })
    }
}

fn parse_worktrees(output: &str) -> Result<Vec<WorktreeInfo>> {
    let mut worktrees = Vec::new();
    for block in output.replace("\r\n", "\n").split("\n\n") {
        let mut path = None;
        let mut head = None;
        let mut branch = None;
        let mut locked = false;
        let mut prunable = false;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(value));
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                head = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("branch ") {
                branch = Some(value.trim_start_matches("refs/heads/").to_owned());
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                prunable = true;
            }
        }
        if let Some(path) = path {
            worktrees.push(WorktreeInfo {
                path,
                head,
                branch,
                locked,
                prunable,
            });
        }
    }
    if worktrees.is_empty() && !output.trim().is_empty() {
        bail!("git worktree output did not contain a worktree record");
    }
    Ok(worktrees)
}

fn slug(title: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !result.is_empty() {
            result.push('-');
            separator = true;
        }
        if result.len() >= 40 {
            break;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "task".to_owned()
    } else {
        result.to_owned()
    }
}

fn git_output(root: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(external_path(root))
        .args(arguments)
        .output()
        .with_context(|| format!("run git in {}", root.display()))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git output is not UTF-8")
}

fn git_success(root: &Path, arguments: &[&str]) -> Result<()> {
    git_output(root, arguments).map(|_| ())
}

fn quote(path: &Path) -> String {
    format!("\"{}\"", external_path(path).replace('"', "\\\""))
}

fn external_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_worktrees, slug};

    #[test]
    fn parses_paths_with_spaces_and_task_slugs_are_bounded() {
        let worktrees =
            parse_worktrees("worktree C:/repo with spaces\nHEAD abc\nbranch refs/heads/main\n\n")
                .expect("parse");
        assert_eq!(worktrees[0].path.to_string_lossy(), "C:/repo with spaces");
        assert_eq!(slug("  OAuth callback + PKCE  "), "oauth-callback-pkce");
    }
}
