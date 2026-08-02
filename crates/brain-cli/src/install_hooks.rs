use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use atomicwrites::{AllowOverwrite, AtomicFile};

const SESSION_MATCHER: &str = "startup|resume|clear|compact|fork";

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct HookInstallResult {
    pub changed: bool,
    pub settings_path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

pub fn install_claude_hooks(
    settings_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let settings_path = settings_path.as_ref();
    let hook_executable = canonical_hook_executable(hook_executable.as_ref())?;
    let mut settings = read_settings(settings_path)?;
    validate_settings_shape(&settings)?;
    if contains_owned_hook(&settings, &hook_executable) {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: settings_path.to_path_buf(),
            backup_path: None,
        });
    }

    let root = settings
        .as_object_mut()
        .expect("settings shape was validated as an object");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("hooks shape was validated as an object");
    let session_start = hooks
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Claude hooks.SessionStart must be an array")?;
    session_start.push(serde_json::json!({
        "matcher": SESSION_MATCHER,
        "hooks": [{
            "type": "command",
            "command": hook_executable,
            "args": ["--harness", "claude-code"],
            "timeout": 1
        }]
    }));

    replace_with_backup(settings_path, &settings)
}

pub fn uninstall_claude_hooks(
    settings_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let settings_path = settings_path.as_ref();
    let hook_executable = canonical_hook_executable(hook_executable.as_ref())?;
    let mut settings = read_settings(settings_path)?;
    validate_settings_shape(&settings)?;
    let mut changed = false;

    if let Some(root) = settings.as_object_mut()
        && let Some(hooks_value) = root.get_mut("hooks")
    {
        let hooks = hooks_value
            .as_object_mut()
            .expect("hooks shape was validated as an object");
        if let Some(groups_value) = hooks.get_mut("SessionStart") {
            let groups = groups_value
                .as_array_mut()
                .context("Claude hooks.SessionStart must be an array")?;
            for group in groups.iter_mut() {
                let Some(commands) = group
                    .get_mut("hooks")
                    .and_then(serde_json::Value::as_array_mut)
                else {
                    continue;
                };
                let before = commands.len();
                commands.retain(|command| !is_owned_command(command, &hook_executable));
                changed |= before != commands.len();
            }
            groups.retain(|group| {
                group
                    .get("hooks")
                    .and_then(serde_json::Value::as_array)
                    .is_none_or(|commands| !commands.is_empty())
            });
            if groups.is_empty() {
                hooks.remove("SessionStart");
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    if !changed {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: settings_path.to_path_buf(),
            backup_path: None,
        });
    }
    replace_with_backup(settings_path, &settings)
}

fn canonical_hook_executable(path: &Path) -> Result<PathBuf> {
    if !path.is_file() {
        bail!("brain hook executable does not exist: {}", path.display());
    }
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("resolve brain hook executable {}", path.display()))?;
    Ok(without_windows_verbatim_prefix(canonical))
}

fn without_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    text.strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}

fn read_settings(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("read Claude settings {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse Claude settings {}", path.display()))
}

fn validate_settings_shape(settings: &serde_json::Value) -> Result<()> {
    let Some(root) = settings.as_object() else {
        bail!("Claude settings must be a JSON object");
    };
    if let Some(hooks) = root.get("hooks") {
        let Some(hooks) = hooks.as_object() else {
            bail!("Claude settings hooks must be a JSON object");
        };
        if let Some(session_start) = hooks.get("SessionStart")
            && !session_start.is_array()
        {
            bail!("Claude hooks.SessionStart must be an array");
        }
    }
    Ok(())
}

fn contains_owned_hook(settings: &serde_json::Value, hook_executable: &Path) -> bool {
    settings
        .pointer("/hooks/SessionStart")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
        .flatten()
        .any(|command| is_owned_command(command, hook_executable))
}

fn is_owned_command(command: &serde_json::Value, hook_executable: &Path) -> bool {
    command.get("type").and_then(serde_json::Value::as_str) == Some("command")
        && command
            .get("command")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|candidate| path_text_eq(candidate, hook_executable))
        && command.get("args") == Some(&serde_json::json!(["--harness", "claude-code"]))
}

fn path_text_eq(candidate: &str, expected: &Path) -> bool {
    candidate.replace('/', "\\").to_lowercase()
        == expected.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn replace_with_backup(path: &Path, settings: &serde_json::Value) -> Result<HookInstallResult> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create Claude settings directory {}", parent.display()))?;
    }
    let backup_path = if path.exists() {
        let backup = backup_path(path);
        std::fs::copy(path, &backup).with_context(|| {
            format!(
                "back up Claude settings {} to {}",
                path.display(),
                backup.display()
            )
        })?;
        Some(backup)
    } else {
        None
    };
    let bytes = serde_json::to_vec_pretty(settings)?;
    let _: serde_json::Value = serde_json::from_slice(&bytes)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .with_context(|| format!("atomically replace Claude settings {}", path.display()))?;
    Ok(HookInstallResult {
        changed: true,
        settings_path: path.to_path_buf(),
        backup_path,
    })
}

fn backup_path(settings_path: &Path) -> PathBuf {
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    let filename = settings_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    settings_path.with_file_name(format!("{filename}.agent-brain-backup-{timestamp}.json"))
}
