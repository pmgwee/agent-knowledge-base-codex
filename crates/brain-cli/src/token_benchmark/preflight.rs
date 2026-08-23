use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::ProjectId;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::artifacts::sha256;
use super::{BenchmarkCondition, BenchmarkHarness};

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
    #[serde(default)]
    pub claude_mcp: Option<String>,
    pub codex_hooks: Option<String>,
    pub codex_config: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ConditionProfileValidation {
    pub valid: bool,
    pub profile_sha256: BTreeMap<String, String>,
    pub diffs: BTreeMap<String, ConditionDiff>,
    pub errors: Vec<String>,
}

pub fn validate_condition_profiles(directory: &Path) -> Result<ConditionProfileValidation> {
    ensure!(directory.is_dir(), "condition profile directory is missing");
    let names = profile_names();
    let mut profiles = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    let mut errors = Vec::new();
    for (harness, condition, name) in &names {
        let path = directory.join(name);
        let bytes = fs::read(&path).with_context(|| format!("read profile {}", path.display()))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        hashes.insert((*name).to_owned(), sha256(&bytes));
        validate_profile_shape(&value, *harness, *condition, name, &mut errors);
        profiles.insert((*harness, *condition), value);
    }

    let invariant_paths = [
        "/schema_version",
        "/model",
        "/effort",
        "/permissions",
        "/native_memory",
        "/skills",
        "/providers",
        "/instructions",
    ];
    for harness in BenchmarkHarness::ALL {
        let baseline = &profiles[&(harness, BenchmarkCondition::C0)];
        for condition in BenchmarkCondition::ALL.into_iter().skip(1) {
            let candidate = &profiles[&(harness, condition)];
            for pointer in invariant_paths {
                if baseline.pointer(pointer) != candidate.pointer(pointer) {
                    errors.push(format!(
                        "{} {} changes invariant {}",
                        harness.as_str(),
                        condition.as_str(),
                        pointer.trim_start_matches('/')
                    ));
                }
            }
        }
    }

    let mut diffs = BTreeMap::new();
    for harness in BenchmarkHarness::ALL {
        for (baseline, treatment, allowed) in [
            (
                BenchmarkCondition::C0,
                BenchmarkCondition::C1,
                vec!["/condition", "/tools/codegraph"],
            ),
            (
                BenchmarkCondition::C1,
                BenchmarkCondition::C2,
                vec![
                    "/condition",
                    "/hooks/agent_brain",
                    "/environment/BRAIN_HOME",
                    "/environment/BRAIN_PIPE_NAME",
                ],
            ),
            (
                BenchmarkCondition::C2,
                BenchmarkCondition::C3,
                vec!["/condition", "/hooks/agent_brain/events"],
            ),
            (
                BenchmarkCondition::C3,
                BenchmarkCondition::C4,
                vec!["/condition", "/mcp/agent_brain"],
            ),
        ] {
            let diff = compare_condition_configs(
                &profiles[&(harness, baseline)],
                &profiles[&(harness, treatment)],
                &allowed,
            );
            if !diff.passed {
                errors.push(format!(
                    "{} {} vs {} has unexpected differences: {}",
                    harness.as_str(),
                    treatment.as_str(),
                    baseline.as_str(),
                    diff.unexpected_paths.join(", ")
                ));
            }
            diffs.insert(
                format!(
                    "{}-{}_vs_{}",
                    harness.as_str(),
                    treatment.as_str(),
                    baseline.as_str()
                ),
                diff,
            );
        }
    }
    Ok(ConditionProfileValidation {
        valid: errors.is_empty(),
        profile_sha256: hashes,
        diffs,
        errors,
    })
}

fn validate_profile_shape(
    value: &Value,
    harness: BenchmarkHarness,
    condition: BenchmarkCondition,
    name: &str,
    errors: &mut Vec<String>,
) {
    let expected_harness = harness.as_str();
    if value.pointer("/schema_version").and_then(Value::as_u64) != Some(2)
        || value.pointer("/harness").and_then(Value::as_str) != Some(expected_harness)
        || value.pointer("/condition").and_then(Value::as_str) != Some(condition.as_str())
    {
        errors.push(format!("{name} identity does not match its filename"));
    }
    for field in ["model", "effort", "permissions", "native_memory", "skills"] {
        if value.get(field).and_then(Value::as_str) != Some("native_default") {
            errors.push(format!("{name} {field} is not native_default"));
        }
    }
    if value.get("providers") != Some(&serde_json::json!({})) {
        errors.push(format!("{name} enables an undeclared provider"));
    }
    let expected_codegraph = if condition == BenchmarkCondition::C0 {
        serde_json::json!({"enabled": false})
    } else {
        serde_json::json!({
            "enabled": true,
            "command": "codegraph",
            "state": "{{sample_codegraph_home}}"
        })
    };
    if value.pointer("/tools/codegraph") != Some(&expected_codegraph) {
        errors.push(format!("{name} has the wrong CodeGraph reachability"));
    }
    let events = match condition {
        BenchmarkCondition::C0 | BenchmarkCondition::C1 => None,
        BenchmarkCondition::C2 => Some(serde_json::json!(["SessionStart", "SessionEnd"])),
        BenchmarkCondition::C3 | BenchmarkCondition::C4 => Some(serde_json::json!([
            "SessionStart",
            "SessionEnd",
            "UserPromptSubmit"
        ])),
    };
    match events {
        None if value.get("hooks") != Some(&serde_json::json!({})) => {
            errors.push(format!("{name} exposes a Brain hook before c2"));
        }
        Some(events) => {
            if value.pointer("/hooks/agent_brain/events") != Some(&events)
                || value
                    .pointer("/hooks/agent_brain/transport")
                    .and_then(Value::as_str)
                    != Some("{{sample_brain_home}}/bin/brain-hook.exe")
            {
                errors.push(format!("{name} has the wrong Brain hook lifecycle"));
            }
        }
        _ => {}
    }
    let expected_mcp = if condition == BenchmarkCondition::C4 {
        serde_json::json!({
            "agent_brain": {
                "transport": "{{sample_brain_home}}/bin/brain-mcp.exe",
                "scope": "depth_on_demand"
            }
        })
    } else {
        serde_json::json!({})
    };
    if value.get("mcp") != Some(&expected_mcp) {
        errors.push(format!("{name} has the wrong Brain MCP reachability"));
    }
    let expected_environment = if condition.requires_brain_service() {
        serde_json::json!({
            "BRAIN_HOME": "{{sample_brain_home}}",
            "BRAIN_PIPE_NAME": "{{pipe_name}}"
        })
    } else {
        serde_json::json!({})
    };
    if value.get("environment") != Some(&expected_environment) {
        errors.push(format!("{name} exposes the wrong Brain endpoint"));
    }
}

fn profile_names() -> Vec<(BenchmarkHarness, BenchmarkCondition, &'static str)> {
    vec![
        (
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C0,
            "claude-c0-native-default.json",
        ),
        (
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C1,
            "claude-c1-codegraph.json",
        ),
        (
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C2,
            "claude-c2-startup.json",
        ),
        (
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C3,
            "claude-c3-prompt-push.json",
        ),
        (
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::C4,
            "claude-c4-full-brain.json",
        ),
        (
            BenchmarkHarness::Codex,
            BenchmarkCondition::C0,
            "codex-c0-native-default.json",
        ),
        (
            BenchmarkHarness::Codex,
            BenchmarkCondition::C1,
            "codex-c1-codegraph.json",
        ),
        (
            BenchmarkHarness::Codex,
            BenchmarkCondition::C2,
            "codex-c2-startup.json",
        ),
        (
            BenchmarkHarness::Codex,
            BenchmarkCondition::C3,
            "codex-c3-prompt-push.json",
        ),
        (
            BenchmarkHarness::Codex,
            BenchmarkCondition::C4,
            "codex-c4-full-brain.json",
        ),
    ]
}

pub(crate) fn condition_profile_name(
    harness: BenchmarkHarness,
    condition: BenchmarkCondition,
) -> &'static str {
    profile_names()
        .into_iter()
        .find(|(candidate_harness, candidate_condition, _)| {
            *candidate_harness == harness && *candidate_condition == condition
        })
        .map(|(_, _, name)| name)
        .expect("all benchmark profiles are registered")
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
