use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use atomicwrites::{AllowOverwrite, AtomicFile};

const SESSION_MATCHER: &str = "startup|resume|clear|compact|fork";
const CODEX_SESSION_MATCHER: &str = "^(startup|resume|clear|compact)$";

/// Outer hook budget (seconds) the *harness* grants the hook process before killing it.
/// Must comfortably exceed `brain_hook::HOOK_HARD_TIMEOUT` (the hook's own internal fail-open
/// deadline), or the harness kills the hook before it can fail open on its own. Generous on
/// purpose: the normal path returns in ~150 ms, and this only bites if the hook process itself
/// hangs — strictly worse than a slow service.
const CLAUDE_HOOK_TIMEOUT_SECONDS: u64 = 10;
const CODEX_HOOK_TIMEOUT_SECONDS: u64 = 15;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct HookInstallResult {
    pub changed: bool,
    pub settings_path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

/// What the install found for brain-hook entries in a harness config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookPresence {
    /// No brain-hook entry exists.
    Missing,
    /// The only brain-hook entry points at the current executable.
    CurrentOnly,
    /// A brain-hook entry exists at a different path, with or without the current one.
    StalePresent,
}

pub fn install_claude_hooks(
    settings_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let settings_path = settings_path.as_ref();
    let hook_executable = canonical_hook_executable(hook_executable.as_ref())?;
    let mut settings = read_json_document(settings_path, "Claude settings")?;
    validate_hooks_shape(&settings, "Claude settings")?;

    // Idempotency keys off the binary *name*, not the path. Re-pointing the hook (e.g.
    // target/ -> ~/AgentBrain/bin/) must REPLACE the stale entry rather than append a second
    // one — a duplicate double-fires on every session start and doubles load on a pipe that
    // serves one client at a time.
    if claude_hook_presence(&settings, &hook_executable) == HookPresence::CurrentOnly {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: settings_path.to_path_buf(),
            backup_path: None,
        });
    }

    strip_claude_brain_hooks(&mut settings);
    push_claude_hook_entry(&mut settings, &hook_executable)?;
    replace_with_backup(settings_path, &settings, "Claude settings")
}

pub fn uninstall_claude_hooks(
    settings_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let settings_path = settings_path.as_ref();
    let _ = canonical_hook_executable(hook_executable.as_ref())?;
    let mut settings = read_json_document(settings_path, "Claude settings")?;
    validate_hooks_shape(&settings, "Claude settings")?;

    // Removal is also keyed by binary name: an uninstall must clean up a brain-hook entry no
    // matter which path it was registered at, including paths left over from before a
    // re-point. The executable argument is still required (its existence is the contract that
    // the caller owns a real brain install) but does not select which entries are removed.
    if !strip_claude_brain_hooks(&mut settings) {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: settings_path.to_path_buf(),
            backup_path: None,
        });
    }
    replace_with_backup(settings_path, &settings, "Claude settings")
}

pub fn install_codex_hooks(
    hooks_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let hooks_path = hooks_path.as_ref();
    let hook_executable = canonical_hook_executable(hook_executable.as_ref())?;
    let mut document = read_json_document(hooks_path, "Codex hooks")?;
    validate_hooks_shape(&document, "Codex hooks")?;

    if codex_hook_presence(&document, &hook_executable) == HookPresence::CurrentOnly {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: hooks_path.to_path_buf(),
            backup_path: None,
        });
    }

    strip_codex_brain_hooks(&mut document);
    push_codex_hook_entry(&mut document, &hook_executable)?;
    replace_with_backup(hooks_path, &document, "Codex hooks")
}

pub fn uninstall_codex_hooks(
    hooks_path: impl AsRef<Path>,
    hook_executable: impl AsRef<Path>,
) -> Result<HookInstallResult> {
    let hooks_path = hooks_path.as_ref();
    let _ = canonical_hook_executable(hook_executable.as_ref())?;
    let mut document = read_json_document(hooks_path, "Codex hooks")?;
    validate_hooks_shape(&document, "Codex hooks")?;

    if !strip_codex_brain_hooks(&mut document) {
        return Ok(HookInstallResult {
            changed: false,
            settings_path: hooks_path.to_path_buf(),
            backup_path: None,
        });
    }
    replace_with_backup(hooks_path, &document, "Codex hooks")
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

fn read_json_document(path: &Path, label: &str) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let bytes = std::fs::read(path).with_context(|| format!("read {label} {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {label} {}", path.display()))
}

fn validate_hooks_shape(settings: &serde_json::Value, label: &str) -> Result<()> {
    let Some(root) = settings.as_object() else {
        bail!("{label} must be a JSON object");
    };
    if let Some(hooks) = root.get("hooks") {
        let Some(hooks) = hooks.as_object() else {
            bail!("{label} hooks must be a JSON object");
        };
        if let Some(session_start) = hooks.get("SessionStart")
            && !session_start.is_array()
        {
            bail!("{label} hooks.SessionStart must be an array");
        }
    }
    Ok(())
}

fn claude_hook_presence(settings: &serde_json::Value, hook_executable: &Path) -> HookPresence {
    let mut has_current = false;
    let mut has_stale = false;
    for command in brain_hook_commands(settings) {
        let Some(candidate) = command.get("command").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if path_text_eq(candidate, hook_executable) {
            has_current = true;
        } else {
            has_stale = true;
        }
    }
    match (has_current, has_stale) {
        (true, false) => HookPresence::CurrentOnly,
        (false, false) => HookPresence::Missing,
        _ => HookPresence::StalePresent,
    }
}

fn codex_hook_presence(document: &serde_json::Value, hook_executable: &Path) -> HookPresence {
    let expected = codex_command(hook_executable);
    let mut has_current = false;
    let mut has_stale = false;
    for command in brain_hook_commands(document) {
        let is_current = ["commandWindows", "command"]
            .into_iter()
            .filter_map(|key| command.get(key).and_then(serde_json::Value::as_str))
            .any(|candidate| command_text_eq(candidate, &expected));
        if is_current {
            has_current = true;
        } else {
            has_stale = true;
        }
    }
    match (has_current, has_stale) {
        (true, false) => HookPresence::CurrentOnly,
        (false, false) => HookPresence::Missing,
        _ => HookPresence::StalePresent,
    }
}

/// Iterate every command registered under `hooks.SessionStart`, regardless of group.
fn brain_hook_commands(document: &serde_json::Value) -> Vec<&serde_json::Value> {
    document
        .pointer("/hooks/SessionStart")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|command| command_invokes_brain_hook(command))
        .collect()
}

/// True if this command entry invokes the brain-hook binary under *either* harness, keyed by
/// the binary name (`brain-hook`) rather than its full path so a re-point is detectable.
fn command_invokes_brain_hook(command: &serde_json::Value) -> bool {
    if command.get("type").and_then(serde_json::Value::as_str) != Some("command") {
        return false;
    }
    let is_claude = command.get("args") == Some(&serde_json::json!(["--harness", "claude-code"]));
    let codex_harness_in_text = ["commandWindows", "command"]
        .into_iter()
        .filter_map(|key| command.get(key).and_then(serde_json::Value::as_str))
        .any(|text| text.to_lowercase().contains("--harness codex"));
    if !(is_claude || codex_harness_in_text) {
        return false;
    }
    ["commandWindows", "command"]
        .into_iter()
        .filter_map(|key| command.get(key).and_then(serde_json::Value::as_str))
        .any(points_at_brain_hook)
}

/// Extract the executable token from a command string (handling a quoted Windows path with
/// trailing args) and report whether its file stem is `brain-hook`.
fn points_at_brain_hook(command_text: &str) -> bool {
    let trimmed = command_text.trim();
    let executable = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        trimmed.split_whitespace().next().unwrap_or(trimmed)
    };
    Path::new(executable)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("brain-hook"))
}

/// Remove every brain-hook command under `hooks.SessionStart`. Returns whether anything was
/// removed. Cleans up the now-empty structures to keep the document tidy.
fn strip_claude_brain_hooks(settings: &mut serde_json::Value) -> bool {
    strip_brain_hooks(settings, command_invokes_brain_hook)
}

fn strip_codex_brain_hooks(document: &mut serde_json::Value) -> bool {
    strip_brain_hooks(document, command_invokes_brain_hook)
}

fn strip_brain_hooks(
    document: &mut serde_json::Value,
    owns: fn(&serde_json::Value) -> bool,
) -> bool {
    let Some(root) = document.as_object_mut() else {
        return false;
    };
    let Some(hooks_value) = root.get_mut("hooks") else {
        return false;
    };
    let Some(hooks) = hooks_value.as_object_mut() else {
        return false;
    };
    let Some(groups_value) = hooks.get_mut("SessionStart") else {
        return false;
    };
    let Some(groups) = groups_value.as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for group in groups.iter_mut() {
        let Some(commands) = group
            .get_mut("hooks")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        let before = commands.len();
        commands.retain(|command| !owns(command));
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
    if hooks.is_empty() {
        root.remove("hooks");
    }
    changed
}

fn push_claude_hook_entry(settings: &mut serde_json::Value, hook_executable: &Path) -> Result<()> {
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
            "timeout": CLAUDE_HOOK_TIMEOUT_SECONDS
        }]
    }));
    Ok(())
}

fn push_codex_hook_entry(document: &mut serde_json::Value, hook_executable: &Path) -> Result<()> {
    let root = document
        .as_object_mut()
        .expect("Codex hook document was validated as an object");
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("Codex hooks were validated as an object");
    let session_start = hooks
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Codex hooks.SessionStart must be an array")?;
    let command = codex_command(hook_executable);
    session_start.push(serde_json::json!({
        "matcher": CODEX_SESSION_MATCHER,
        "hooks": [{
            "type": "command",
            "command": command,
            "commandWindows": command,
            "timeout": CODEX_HOOK_TIMEOUT_SECONDS,
            "statusMessage": "Loading project memory",
            "additionalContextLimit": 1500
        }]
    }));
    Ok(())
}

fn codex_command(hook_executable: &Path) -> String {
    format!("\"{}\" --harness codex", hook_executable.display())
}

fn command_text_eq(candidate: &str, expected: &str) -> bool {
    candidate.replace('/', "\\").to_lowercase() == expected.replace('/', "\\").to_lowercase()
}

fn path_text_eq(candidate: &str, expected: &Path) -> bool {
    candidate.replace('/', "\\").to_lowercase()
        == expected.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn replace_with_backup(
    path: &Path,
    settings: &serde_json::Value,
    label: &str,
) -> Result<HookInstallResult> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {label} directory {}", parent.display()))?;
    }
    let backup_path = if path.exists() {
        let backup = backup_path(path);
        std::fs::copy(path, &backup).with_context(|| {
            format!("back up {label} {} to {}", path.display(), backup.display())
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
        .with_context(|| format!("atomically replace {label} {}", path.display()))?;
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
