use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};

/// The hook events the brain registers, in both harnesses.
///
/// `SessionEnd` is not decoration. Transcripts are append-only JSONL, so a session ending writes
/// no line — the file simply stops growing — and `EventType::SessionEnded` had therefore never
/// been emitted once across 139,192 captured events. `ConsolidationReason::SessionStopped` was
/// unreachable code that read as wired up, and a session's work waited for the *next* session to
/// push it over the 200-event threshold. This hook is the only thing that can observe the boundary.
const BRAIN_HOOK_EVENTS: [&str; 3] = ["SessionStart", "SessionEnd", "UserPromptSubmit"];

/// Events registered for Claude Code.
///
/// `UserPromptSubmit` carries the mid-session push: the brain used to hand over context **once**, at
/// session start, so a session that ran for hours and changed subject was never re-oriented. Other
/// tools already sit on this hook here — CodeGraph does — and the installer only ever strips entries
/// whose binary is `brain-hook`, so theirs survives ours.
const CLAUDE_EVENTS: [&str; 3] = ["SessionStart", "SessionEnd", "UserPromptSubmit"];

/// Events registered for Codex.
///
/// Now the same three as Claude. `UserPromptSubmit` was withheld while Codex appeared to dispatch
/// nothing — registering an event a harness never fires looks exactly like a shipped feature that
/// silently does not run. Codex Desktop was verified dispatching `SessionStart` on 9 August 2026
/// (build 26.803.41515, CLI 0.147.0), so the premise is gone and the asymmetry it protected is not
/// worth keeping: without this hook Codex orients once per session while Claude re-orients on every
/// prompt.
const CODEX_EVENTS: [&str; 3] = ["SessionStart", "SessionEnd", "UserPromptSubmit"];

const SESSION_MATCHER: &str = "startup|resume|clear|compact|fork";

/// `SessionEnd` fires on `clear`, `logout`, `prompt_input_exit` and `other`, and every one of them
/// is a real boundary — so this matches all of them rather than enumerating a subset that would
/// silently miss the way most sessions actually end.
const SESSION_END_MATCHER: &str = ".*";
const CODEX_SESSION_MATCHER: &str = "^(startup|resume|clear|compact)$";

/// Outer hook budget (seconds) the *harness* grants the hook process before killing it.
/// Must comfortably exceed `brain_hook::HOOK_HARD_TIMEOUT` (the hook's own internal fail-open
/// deadline), or the harness kills the hook before it can fail open on its own. Generous on
/// purpose: the normal path returns in ~150 ms, and this only bites if the hook process itself
/// hangs — strictly worse than a slow service.
const CLAUDE_HOOK_TIMEOUT_SECONDS: u64 = 10;
const CODEX_HOOK_TIMEOUT_SECONDS: u64 = 15;

/// Codex's ceiling for `SessionEnd`, which it clamps rather than honours.
///
/// Anything larger produces `⚠ clamping SessionEnd hook timeout to 3s` on every load. The warning
/// is harmless and the behaviour is right — a session that is *ending* cannot be kept waiting —
/// but emitting a number the harness will silently overrule is how a config drifts away from what
/// actually runs. Write what Codex will use.
///
/// It is comfortably above the hook's own `HOOK_HARD_TIMEOUT`-bounded round trip (~150 ms healthy),
/// and `SessionEnd` carries no orientation back — it only records a boundary — so the budget is not
/// doing the work it does at session start.
const CODEX_SESSION_END_TIMEOUT_SECONDS: u64 = 3;

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
    // `commandWindows` cannot quote the executable — see `codex_command_windows` — so a path with a
    // space in it would produce a hook that cannot launch. Refuse loudly here rather than write one
    // that fails silently, which is the exact shape of the defect this rule exists to prevent.
    ensure!(
        !hook_executable.to_string_lossy().contains(' '),
        "Codex hooks cannot launch an executable whose path contains a space: {}. Codex's          commandWindows field does not strip quotes, so quoting it fails with `hook exited with          code 1`. Install brain-hook.exe somewhere without spaces — ~/AgentBrain/bin is the default.",
        hook_executable.display()
    );
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
        // Complete only when every event is covered. A config carrying just the old `SessionStart`
        // registration is stale in the sense that matters: it is missing one.
        (true, false)
            if registered_event_count(settings, &CLAUDE_EVENTS) == CLAUDE_EVENTS.len() =>
        {
            HookPresence::CurrentOnly
        }
        (true, false) => HookPresence::StalePresent,
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
        // Same completeness rule as Claude: a config missing one of the events is not current.
        (true, false) if registered_event_count(document, &CODEX_EVENTS) == CODEX_EVENTS.len() => {
            HookPresence::CurrentOnly
        }
        (true, false) => HookPresence::StalePresent,
        (false, false) => HookPresence::Missing,
        _ => HookPresence::StalePresent,
    }
}

/// Iterate every brain-hook command registered under any of [`BRAIN_HOOK_EVENTS`].
///
/// Spanning both events is what makes the presence check honest after this gained `SessionEnd`: a
/// config holding only the old `SessionStart` entry must read as incomplete, or an upgrade would
/// see the entry it already had, report "no change", and leave session boundaries unobserved.
fn brain_hook_commands(document: &serde_json::Value) -> Vec<&serde_json::Value> {
    BRAIN_HOOK_EVENTS
        .into_iter()
        .filter_map(|event| document.pointer(&format!("/hooks/{event}")))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|command| command_invokes_brain_hook(command))
        .collect()
}

/// How many of [`BRAIN_HOOK_EVENTS`] carry a brain-hook entry.
fn registered_event_count(document: &serde_json::Value, events: &[&str]) -> usize {
    events
        .iter()
        .copied()
        .filter(|event| {
            document
                .pointer(&format!("/hooks/{event}"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
                .flatten()
                .any(command_invokes_brain_hook)
        })
        .count()
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
    let mut changed = false;
    for event in BRAIN_HOOK_EVENTS {
        let Some(groups) = hooks
            .get_mut(event)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
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
            hooks.remove(event);
        }
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
    let session_end = hooks
        .entry("SessionEnd")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Claude hooks.SessionEnd must be an array")?;
    session_end.push(serde_json::json!({
        "matcher": SESSION_END_MATCHER,
        "hooks": [{
            "type": "command",
            "command": hook_executable,
            "args": ["--harness", "claude-code"],
            "timeout": CLAUDE_HOOK_TIMEOUT_SECONDS
        }]
    }));
    let prompt_submit = hooks
        .entry("UserPromptSubmit")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Claude hooks.UserPromptSubmit must be an array")?;
    prompt_submit.push(serde_json::json!({
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
    let command_windows = codex_command_windows(hook_executable);
    session_start.push(serde_json::json!({
        "matcher": CODEX_SESSION_MATCHER,
        "hooks": [{
            "type": "command",
            "command": command,
            "commandWindows": command_windows,
            "timeout": CODEX_HOOK_TIMEOUT_SECONDS,
            "statusMessage": "Loading project memory",
            "additionalContextLimit": 1500
        }]
    }));
    let session_end = hooks
        .entry("SessionEnd")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Codex hooks.SessionEnd must be an array")?;
    session_end.push(serde_json::json!({
        "matcher": SESSION_END_MATCHER,
        "hooks": [{
            "type": "command",
            "command": command,
            "commandWindows": command_windows,
            "timeout": CODEX_SESSION_END_TIMEOUT_SECONDS
        }]
    }));
    // Mid-session push, now that the premise for withholding it is gone.
    //
    // This was deliberately not registered while Codex appeared to fire nothing — registering an
    // event a harness never dispatches looks exactly like a shipped feature that silently does not
    // run, which is the failure this project keeps finding. Codex now demonstrably dispatches, so
    // the reasoning no longer applies and the asymmetry it protected is worth closing: without it
    // Codex orients once per session while Claude re-orients on every prompt.
    let prompt_submit = hooks
        .entry("UserPromptSubmit")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Codex hooks.UserPromptSubmit must be an array")?;
    prompt_submit.push(serde_json::json!({
        "matcher": ".*",
        "hooks": [{
            "type": "command",
            "command": command,
            "commandWindows": command_windows,
            "timeout": CODEX_HOOK_TIMEOUT_SECONDS
        }]
    }));
    Ok(())
}

/// The POSIX form, where quoting the executable is correct and necessary.
fn codex_command(hook_executable: &Path) -> String {
    format!("\"{}\" --harness codex", hook_executable.display())
}

/// The Windows form — **unquoted**, and this is the whole reason Codex hooks appeared not to work.
///
/// Codex on Windows executes `commandWindows` in a way that does not strip a quoted executable, so
/// `"C:\…\brain-hook.exe" --harness codex` fails with `hook exited with code 1` while
/// `C:\…\brain-hook.exe --harness codex` runs. The installer emitted the quoted string for both
/// fields, so **every Codex install this tool has ever produced had a hook that could not launch.**
///
/// It cost two wrong conclusions five days apart — first "Codex Desktop does not implement hooks",
/// then an upstream regression (`openai/codex#21639`) with a matching build number, which made a
/// guess look like a diagnosis. Both were reached from zero deliveries *and* zero spool entries,
/// reasoning that a hook which fired and failed would still spool. It does spool — but a hook that
/// **cannot launch** never reaches our binary at all, so it neither delivers nor spools. That
/// observation is indistinguishable from "never invoked", and we read it as the latter twice.
///
/// The trade-off this accepts: an executable path containing a space would now break. That is the
/// correct call here — the path is ours, it is `~/AgentBrain/bin/brain-hook.exe`, and a launcher
/// that never launches is worse than one with a documented constraint. `install_codex_hooks`
/// refuses such a path rather than writing a hook that silently cannot run.
fn codex_command_windows(hook_executable: &Path) -> String {
    format!("{} --harness codex", hook_executable.display())
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
