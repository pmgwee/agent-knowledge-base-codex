use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::ProjectId;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::artifacts::sha256;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ConditionDiff {
    pub passed: bool,
    pub allowed_paths: Vec<String>,
    pub changed_paths: Vec<String>,
    pub unexpected_paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct FrozenSnapshot {
    pub project_id: ProjectId,
    pub path: PathBuf,
    pub sha256: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProductionConfigHashes {
    pub claude_settings: Option<String>,
    pub codex_hooks: Option<String>,
    pub codex_config: Option<String>,
}

pub fn compare_condition_configs(
    control: &Value,
    treatment: &Value,
    allowed_paths: &[&str],
) -> ConditionDiff {
    let mut changed = BTreeSet::new();
    collect_diff(control, treatment, "", &mut changed);
    let allowed: Vec<String> = allowed_paths
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    let unexpected: Vec<_> = changed
        .iter()
        .filter(|path| {
            !allowed
                .iter()
                .any(|allowed| *path == allowed || path.starts_with(&format!("{allowed}/")))
        })
        .cloned()
        .collect();
    ConditionDiff {
        passed: unexpected.is_empty(),
        allowed_paths: allowed,
        changed_paths: changed.into_iter().collect(),
        unexpected_paths: unexpected,
    }
}

pub fn hash_optional_file(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    ensure!(
        path.is_file(),
        "configuration path {} is not a file",
        path.display()
    );
    Ok(Some(sha256(&fs::read(path)?)))
}

pub fn freeze_project_snapshot(
    source: &Path,
    destination: &Path,
    project_id: ProjectId,
) -> Result<FrozenSnapshot> {
    ensure!(
        source.is_dir(),
        "snapshot source {} is not a directory",
        source.display()
    );
    ensure!(!destination.exists(), "snapshot destination already exists");
    let source = source.canonicalize()?;
    let destination_parent = destination
        .parent()
        .context("snapshot destination has no parent")?;
    fs::create_dir_all(destination_parent)?;
    let parent = destination_parent.canonicalize()?;
    ensure!(
        !parent.starts_with(&source),
        "snapshot destination is inside source"
    );
    fs::create_dir(destination)?;
    copy_tree(&source, destination)?;
    let mut files = Vec::new();
    collect_files(destination, destination, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    for (relative, path) in &files {
        let content = fs::read(path)?;
        bytes = bytes.saturating_add(content.len() as u64);
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(&content);
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)?;
    }
    Ok(FrozenSnapshot {
        project_id,
        path: destination.to_path_buf(),
        sha256: hex::encode(digest.finalize()),
        files: files.len(),
        bytes,
    })
}

fn collect_diff(left: &Value, right: &Value, path: &str, changed: &mut BTreeSet<String>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<_> = left.keys().chain(right.keys()).collect();
            for key in keys {
                let child = format!("{path}/{}", escape_pointer(key));
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => collect_diff(left, right, &child, changed),
                    _ => {
                        changed.insert(child);
                    }
                }
            }
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                collect_diff(left, right, &format!("{path}/{index}"), changed);
            }
        }
        _ if left == right => {}
        _ => {
            changed.insert(if path.is_empty() {
                "/".to_owned()
            } else {
                path.to_owned()
            });
        }
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if entry.file_type()?.is_file() {
            files.push((entry.path().strip_prefix(root)?.to_path_buf(), entry.path()));
        }
    }
    Ok(())
}
