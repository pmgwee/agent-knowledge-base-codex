use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PathChange {
    pub status: String,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightBlocker {
    DirtyWorktree,
    MissingMergeBase,
    UnresolvedWhitespaceError,
    MergeAnalysisUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SemanticWarning {
    pub kind: String,
    pub message: String,
    pub heuristic: bool,
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MergePreflight {
    pub repository: PathBuf,
    pub source_ref: String,
    pub source_commit: String,
    pub target_ref: String,
    pub target_commit: String,
    pub merge_base: Option<String>,
    pub source_changes: Vec<PathChange>,
    pub target_changes: Vec<PathChange>,
    pub conflicts: Vec<PathBuf>,
    pub blockers: Vec<PreflightBlocker>,
    pub warnings: Vec<SemanticWarning>,
    pub detached_head: bool,
    pub git_version: String,
    pub merge_tree_exit_code: Option<i32>,
    pub ready: bool,
}

pub fn merge_preflight(
    repository: impl AsRef<Path>,
    source_ref: &str,
    target_ref: &str,
) -> Result<MergePreflight> {
    let repository = std::fs::canonicalize(repository.as_ref()).with_context(|| {
        format!(
            "resolve preflight repository {}",
            repository.as_ref().display()
        )
    })?;
    let git_version = git_text(&repository, &["--version"])?;
    let source_commit = git_text(
        &repository,
        &["rev-parse", "--verify", &format!("{source_ref}^{{commit}}")],
    )?
    .trim()
    .to_owned();
    let target_commit = git_text(
        &repository,
        &["rev-parse", "--verify", &format!("{target_ref}^{{commit}}")],
    )?
    .trim()
    .to_owned();
    let dirty = !git_text(&repository, &["status", "--porcelain"])?
        .trim()
        .is_empty();
    let detached_head = !git(&repository, &["symbolic-ref", "-q", "HEAD"])?
        .status
        .success();
    let merge_base_output = git(&repository, &["merge-base", &source_commit, &target_commit])?;
    let merge_base = merge_base_output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&merge_base_output.stdout)
                .trim()
                .to_owned()
        })
        .filter(|value| !value.is_empty());
    let mut blockers = Vec::new();
    if dirty {
        blockers.push(PreflightBlocker::DirtyWorktree);
    }
    let Some(merge_base) = merge_base else {
        blockers.push(PreflightBlocker::MissingMergeBase);
        return Ok(MergePreflight {
            repository,
            source_ref: source_ref.to_owned(),
            source_commit,
            target_ref: target_ref.to_owned(),
            target_commit,
            merge_base: None,
            source_changes: Vec::new(),
            target_changes: Vec::new(),
            conflicts: Vec::new(),
            blockers,
            warnings: Vec::new(),
            detached_head,
            git_version: git_version.trim().to_owned(),
            merge_tree_exit_code: None,
            ready: false,
        });
    };
    let source_changes = changes(&repository, &merge_base, &source_commit)?;
    let target_changes = changes(&repository, &merge_base, &target_commit)?;
    let whitespace = git(
        &repository,
        &["diff", "--check", &format!("{merge_base}..{source_commit}")],
    )?;
    if !whitespace.status.success() || !whitespace.stdout.is_empty() {
        blockers.push(PreflightBlocker::UnresolvedWhitespaceError);
    }
    let merge_tree = git(
        &repository,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            &source_commit,
            &target_commit,
        ],
    )?;
    let merge_tree_exit_code = merge_tree.status.code();
    let conflicts = match merge_tree_exit_code {
        Some(0) => Vec::new(),
        Some(1) => conflict_paths(&merge_tree, &source_changes, &target_changes),
        _ => {
            blockers.push(PreflightBlocker::MergeAnalysisUnavailable);
            Vec::new()
        }
    };
    let warnings = semantic_warnings(&source_changes, &target_changes, &conflicts);
    let ready = blockers.is_empty() && conflicts.is_empty();
    Ok(MergePreflight {
        repository,
        source_ref: source_ref.to_owned(),
        source_commit,
        target_ref: target_ref.to_owned(),
        target_commit,
        merge_base: Some(merge_base),
        source_changes,
        target_changes,
        conflicts,
        blockers,
        warnings,
        detached_head,
        git_version: git_version.trim().to_owned(),
        merge_tree_exit_code,
        ready,
    })
}

fn changes(repository: &Path, base: &str, commit: &str) -> Result<Vec<PathChange>> {
    let output = git_text(
        repository,
        &[
            "diff",
            "--name-status",
            "--find-renames",
            &format!("{base}..{commit}"),
        ],
    )?;
    let mut changes = Vec::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 2 {
            continue;
        }
        let status = fields[0].to_owned();
        let (old_path, path) = if status.starts_with('R') || status.starts_with('C') {
            if fields.len() < 3 {
                continue;
            }
            (Some(PathBuf::from(fields[1])), PathBuf::from(fields[2]))
        } else {
            (None, PathBuf::from(fields[1]))
        };
        changes.push(PathChange {
            status,
            path,
            old_path,
        });
    }
    Ok(changes)
}

fn conflict_paths(output: &Output, source: &[PathChange], target: &[PathChange]) -> Vec<PathBuf> {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let source_paths = source
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    let target_paths = target
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    let mut conflicts = BTreeSet::new();
    for line in combined.lines() {
        if let Some(path) = line.split(" in ").nth(1)
            && line.contains("CONFLICT")
        {
            conflicts.insert(PathBuf::from(path.trim()));
        }
        if let Some((_, path)) = line.split_once('\t')
            && line.split_whitespace().count() >= 4
        {
            let path = PathBuf::from(path.trim());
            if source_paths.contains(&path) && target_paths.contains(&path) {
                conflicts.insert(path);
            }
        }
    }
    if conflicts.is_empty() {
        conflicts.extend(source_paths.intersection(&target_paths).cloned());
    }
    conflicts.into_iter().collect()
}

fn semantic_warnings(
    source: &[PathChange],
    target: &[PathChange],
    conflicts: &[PathBuf],
) -> Vec<SemanticWarning> {
    let conflicts = conflicts.iter().collect::<BTreeSet<_>>();
    let source_map = source
        .iter()
        .map(|change| (&change.path, change))
        .collect::<BTreeMap<_, _>>();
    let target_map = target
        .iter()
        .map(|change| (&change.path, change))
        .collect::<BTreeMap<_, _>>();
    let mut warnings = Vec::new();
    for path in source_map
        .keys()
        .filter(|path| target_map.contains_key(*path))
    {
        if conflicts.contains(path) {
            continue;
        }
        let value = path.to_string_lossy().to_lowercase();
        let kind = if value.ends_with("lock") || value.ends_with("lock.json") {
            "lockfile_overlap"
        } else if value.contains("migration") {
            "migration_ordering"
        } else {
            "same_path_changed"
        };
        warnings.push(SemanticWarning {
            kind: kind.to_owned(),
            message: format!(
                "Heuristic: both sides changed {}; verify semantic compatibility after Git merge.",
                path.display()
            ),
            heuristic: true,
            paths: vec![(*path).clone()],
        });
    }
    warnings
}

fn git_text(repository: &Path, arguments: &[&str]) -> Result<String> {
    let output = git(repository, arguments)?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git output is not UTF-8")
}

fn git(repository: &Path, arguments: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(external_path(repository))
        .args(arguments)
        .output()
        .with_context(|| format!("execute git in {}", repository.display()))
}

fn external_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}
