use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use brain_adapters::{ClaudeAdapter, CodexAdapter, SourceAdapter};

use crate::{CaptureSupervisor, ServiceLaunchConfig, ServiceProjectConfig, build_capture_bindings};

/// Transcript lines inspected when deciding whether a session belongs to a project.
/// Session metadata carrying `cwd` appears in the first record, so a small bound is
/// enough while keeping the scan cheap across hundreds of files.
const OWNERSHIP_LINE_LIMIT: usize = 64;

/// Where an agent writes its session transcripts.
///
/// Registration resolves these once from the user profile. The service needs them again
/// on every pass, because a session created after registration lives in the same tree
/// but is absent from the stored source list.
#[derive(Clone, Debug)]
pub struct TranscriptRoots {
    pub claude_projects_root: Option<PathBuf>,
    pub codex_sessions_root: Option<PathBuf>,
}

impl TranscriptRoots {
    /// The default per-user locations, matching what `brain register` discovers.
    pub fn from_user_profile() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        match home {
            Some(home) => Self {
                claude_projects_root: Some(home.join(".claude").join("projects")),
                codex_sessions_root: Some(home.join(".codex").join("sessions")),
            },
            None => Self {
                claude_projects_root: None,
                codex_sessions_root: None,
            },
        }
    }

    /// Discovery disabled. Keeps programmatic and test construction bounded.
    pub fn none() -> Self {
        Self {
            claude_projects_root: None,
            codex_sessions_root: None,
        }
    }
}

/// Sources found on disk that the stored configuration does not yet know about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredSources {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
}

impl DiscoveredSources {
    pub fn is_empty(&self) -> bool {
        self.claude.is_empty() && self.codex.is_empty()
    }

    pub fn total(&self) -> usize {
        self.claude.len() + self.codex.len()
    }
}

/// Find transcripts belonging to `project` that are absent from its stored source list.
///
/// Registration writes a point-in-time snapshot of the transcript files that existed when
/// it ran. Every session opened afterwards writes to the same tree but never enters that
/// list, so without this the brain silently stops capturing new work for the project.
///
/// Ownership is decided by the transcript's own `cwd`, exactly as registration decides it,
/// so a session belongs to at most one project and cross-project isolation is preserved.
pub fn discover_new_sources(
    project: &ServiceProjectConfig,
    roots: &TranscriptRoots,
) -> Result<DiscoveredSources> {
    let known = known_paths(project);
    let mut found = DiscoveredSources::default();

    if let Some(root) = roots.claude_projects_root.as_ref()
        && root.is_dir()
    {
        for source in ClaudeAdapter::new(root)
            .discover()
            .context("discover Claude transcripts")?
        {
            if let Some(path) = accept(&source.path, &known, &project.project_root)? {
                found.claude.push(path);
            }
        }
    }

    if let Some(root) = roots.codex_sessions_root.as_ref()
        && root.is_dir()
    {
        for source in CodexAdapter::new(vec![root.clone()])
            .discover()
            .context("discover Codex rollouts")?
        {
            if let Some(path) = accept(&source.path, &known, &project.project_root)? {
                found.codex.push(path);
            }
        }
    }

    found.claude.sort();
    found.codex.sort();
    Ok(found)
}

/// Apply discovered sources to the configuration, returning how many were added.
///
/// Appends only; an existing source is never re-added, reordered, or removed, so cursors
/// stay attached to their sources and previously captured evidence is untouched.
pub fn apply_discovered_sources(
    project: &mut ServiceProjectConfig,
    found: &DiscoveredSources,
) -> usize {
    let before = project.claude_sources.len() + project.codex_sources.len();
    for path in &found.claude {
        if !project.claude_sources.contains(path) {
            project.claude_sources.push(path.clone());
        }
    }
    for path in &found.codex {
        if !project.codex_sources.contains(path) {
            project.codex_sources.push(path.clone());
        }
    }
    project.claude_sources.len() + project.codex_sources.len() - before
}

/// Scan every project in the configuration and append whatever is missing.
///
/// Returns the number of sources added across all projects; zero means the configuration
/// already covers everything on disk, which is the steady state.
pub fn rediscover_all(config: &mut ServiceLaunchConfig, roots: &TranscriptRoots) -> Result<usize> {
    let mut added = 0;
    for project in &mut config.projects {
        let found = discover_new_sources(project, roots)?;
        if !found.is_empty() {
            added += apply_discovered_sources(project, &found);
        }
    }
    Ok(added)
}

/// How often the service rescans for sessions created since the last scan.
///
/// A new session is invisible to capture until the scan that finds it, so this bounds the
/// worst-case delay before a fresh session's work reaches the brain. Two minutes keeps the
/// directory walk negligible while making the gap short enough not to matter in practice.
pub const REDISCOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

/// Watch for transcripts created after the service started, and record them.
///
/// Registration captures a point-in-time snapshot of the transcript files on disk. Without
/// this loop, every session opened afterwards writes to a file the brain never reads — the
/// project silently stops accumulating memory while still reporting healthy.
///
/// The loop only rewrites configuration; it does not restart capture. Newly recorded
/// sources are picked up when the service next starts, and the returned count lets the
/// caller decide whether an immediate rebuild is warranted.
pub async fn run_rediscovery(
    config_path: PathBuf,
    roots: TranscriptRoots,
    supervisor: Arc<CaptureSupervisor>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let mut ticker = tokio::time::interval(REDISCOVERY_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = ticker.tick() => {
                // Never propagate: a discovery failure must not stop capture of the
                // sources already known.
                if let Err(error) = rediscover_and_activate_once(&config_path, &roots, &supervisor).await {
                    tracing::warn!(%error, "source rediscovery failed");
                }
            }
        }
    }
}

pub async fn rediscover_and_activate_once(
    config_path: &Path,
    roots: &TranscriptRoots,
    supervisor: &CaptureSupervisor,
) -> Result<usize> {
    rediscover_once(config_path, roots)?;
    let config = ServiceLaunchConfig::load(config_path)?;
    let activated = supervisor.activate_bindings(build_capture_bindings(&config)?)?;
    if activated > 0 {
        supervisor.capture_once().await?;
        tracing::info!(activated, "activated newly discovered transcript sources");
    }
    Ok(activated)
}

/// One rediscovery pass. Returns how many sources were added.
pub fn rediscover_once(config_path: &Path, roots: &TranscriptRoots) -> Result<usize> {
    let mut config = ServiceLaunchConfig::load(config_path)?;
    let added = rediscover_all(&mut config, roots)?;
    if added > 0 {
        save_config(&config, config_path)?;
        tracing::info!(
            added,
            "recorded transcripts created since the last configuration write"
        );
    }
    Ok(added)
}

/// Persist the configuration after rediscovery.
///
/// Written atomically because the service may be restarted at any moment — a torn config
/// would leave the brain unable to start, which is worse than a missed source.
pub fn save_config(config: &ServiceLaunchConfig, path: &Path) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(config)?;
    atomicwrites::AtomicFile::new(path, atomicwrites::AllowOverwrite)
        .write(|file: &mut std::fs::File| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .map_err(|error| anyhow::anyhow!("{error}"))
        .with_context(|| format!("write service configuration {}", path.display()))
}

fn known_paths(project: &ServiceProjectConfig) -> BTreeSet<String> {
    project
        .claude_sources
        .iter()
        .chain(project.codex_sources.iter())
        .map(|path| normalize(path))
        .collect()
}

/// Decide whether a discovered path is a new source for this project.
///
/// A path is accepted only when it is absent from the known set *and* its transcript
/// claims a working directory inside the project root. Unreadable or ambiguous files are
/// skipped rather than guessed at — a missed source is recoverable on the next pass, but a
/// misattributed one would breach project isolation.
fn accept(path: &Path, known: &BTreeSet<String>, project_root: &Path) -> Result<Option<PathBuf>> {
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => strip_verbatim_prefix(canonical),
        Err(_) => return Ok(None),
    };
    if known.contains(&normalize(&canonical)) {
        return Ok(None);
    }
    if !transcript_claims_project(&canonical, project_root)? {
        return Ok(None);
    }
    Ok(Some(canonical))
}

/// Whether the transcript's recorded working directory sits inside the project root.
///
/// This mirrors registration's ownership test. Reading the transcript's own `cwd` — rather
/// than inferring from the encoded directory name — is what keeps a session bound to the
/// project it actually ran in.
fn transcript_claims_project(transcript: &Path, project_root: &Path) -> Result<bool> {
    let Ok(canonical_root) = std::fs::canonicalize(project_root) else {
        return Ok(false);
    };
    let normalized_root = normalize(&canonical_root);
    let Ok(file) = std::fs::File::open(transcript) else {
        return Ok(false);
    };
    for line in BufReader::new(file).lines().take(OWNERSHIP_LINE_LIMIT) {
        let Ok(line) = line else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let cwd = value
            .get("cwd")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                value
                    .get("payload")
                    .and_then(|payload| payload.get("cwd"))
                    .and_then(serde_json::Value::as_str)
            });
        let Some(cwd) = cwd else { continue };
        let Ok(canonical_cwd) = std::fs::canonicalize(cwd) else {
            continue;
        };
        if is_within(&normalize(&canonical_cwd), &normalized_root) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_within(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Lower-cased, forward-slashed form used only for comparison. Windows paths are
/// case-insensitive and mix separators, so raw string equality would miss real duplicates.
fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy();
    // Strip the Windows verbatim prefix before comparing. Registration stores canonical
    // paths that may carry it while discovery strips it; without this the same file reads
    // as two different sources and is re-added on every pass.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Windows canonicalization prefixes paths with `\\?\`, which is valid to the OS but
/// brittle for third-party launchers and noisy in stored configuration.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}
