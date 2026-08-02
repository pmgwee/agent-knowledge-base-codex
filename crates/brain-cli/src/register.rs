use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_adapters::{ClaudeAdapter, SourceAdapter};
use brain_domain::{ProjectId, ProjectRegistry, WorktreeId};
use brain_service::ServiceLaunchConfig;
use brain_store::EventLedger;

const DISCOVERY_LINE_LIMIT: usize = 64;
const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\agent-brain-v1";

#[derive(Clone, Debug)]
pub struct RegisterOptions {
    pub brain_home: PathBuf,
    pub project_path: PathBuf,
    /// When present, discover Claude JSONL files below this directory. `None`
    /// disables discovery, which keeps programmatic and test registration bounded.
    pub claude_projects_root: Option<PathBuf>,
    pub explicit_claude_sources: Vec<PathBuf>,
    pub pipe_name: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RegistrationResult {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub project_root: PathBuf,
    pub ledger_path: PathBuf,
    pub service_config_path: PathBuf,
    pub claude_sources: Vec<PathBuf>,
}

pub fn register_project(options: RegisterOptions) -> Result<RegistrationResult> {
    let identity = ProjectRegistry::open(&options.brain_home)?
        .register(&options.project_path)
        .context("register project identity")?;
    let ledger_path = options
        .brain_home
        .join("projects")
        .join(identity.project_id.0.to_string())
        .join("evidence")
        .join("hot")
        .join("events.sqlite");
    EventLedger::open(&ledger_path, identity.project_id)
        .context("initialize project evidence ledger")?;

    let mut sources = canonical_sources(&options.explicit_claude_sources)?;
    if let Some(projects_root) = options.claude_projects_root.as_ref()
        && projects_root.is_dir()
    {
        let adapter = ClaudeAdapter::new(projects_root);
        for source in adapter.discover().context("discover Claude transcripts")? {
            if transcript_belongs_to_project(&source.path, &identity.root)? {
                let canonical = std::fs::canonicalize(&source.path).with_context(|| {
                    format!("resolve Claude transcript {}", source.path.display())
                })?;
                sources.push(without_windows_verbatim_prefix(canonical));
            }
        }
    }
    sources.sort_by_key(|path| normalized(path));
    sources.dedup_by(|left, right| normalized(left) == normalized(right));

    let service_config = ServiceLaunchConfig {
        pipe_name: options
            .pipe_name
            .unwrap_or_else(|| DEFAULT_PIPE_NAME.to_owned()),
        project_root: identity.root.clone(),
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        ledger_path: ledger_path.clone(),
        claude_sources: sources.clone(),
    };
    let service_config_path = ServiceLaunchConfig::default_path(&options.brain_home);
    write_json_atomic(&service_config_path, &service_config)?;

    Ok(RegistrationResult {
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        project_root: identity.root,
        ledger_path,
        service_config_path,
        claude_sources: sources,
    })
}

fn canonical_sources(sources: &[PathBuf]) -> Result<Vec<PathBuf>> {
    sources
        .iter()
        .map(|path| {
            if !path.is_file() {
                bail!("Claude transcript does not exist: {}", path.display());
            }
            let canonical = std::fs::canonicalize(path)
                .with_context(|| format!("resolve Claude transcript {}", path.display()))?;
            Ok(without_windows_verbatim_prefix(canonical))
        })
        .collect()
}

fn transcript_belongs_to_project(transcript: &Path, project_root: &Path) -> Result<bool> {
    let reader = BufReader::new(
        File::open(transcript)
            .with_context(|| format!("open Claude transcript {}", transcript.display()))?,
    );
    let canonical_root = std::fs::canonicalize(project_root)
        .with_context(|| format!("resolve project root {}", project_root.display()))?;
    let normalized_root = normalized(&canonical_root);
    for line in reader.lines().take(DISCOVERY_LINE_LIMIT) {
        let Ok(line) = line else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(cwd) = value.get("cwd").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(canonical_cwd) = std::fs::canonicalize(cwd) else {
            continue;
        };
        if is_within(&normalized(&canonical_cwd), &normalized_root) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
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

fn is_within(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create configuration directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let _: serde_json::Value = serde_json::from_slice(&bytes)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .with_context(|| format!("atomically write service configuration {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::transcript_belongs_to_project;

    #[test]
    fn transcript_discovery_uses_structural_cwd_not_encoded_folder_name() {
        let temp = tempfile::tempdir().expect("create discovery fixture");
        let project = temp.path().join("project-with-hyphens");
        let other = temp.path().join("other-project");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(&other).expect("create other project");
        let transcript = temp.path().join("ambiguous-folder-name.jsonl");
        std::fs::write(
            &transcript,
            serde_json::json!({"cwd": project, "type": "user"}).to_string() + "\n",
        )
        .expect("write discovery transcript");

        assert!(
            transcript_belongs_to_project(&transcript, &project).expect("match project transcript")
        );
        assert!(
            !transcript_belongs_to_project(&transcript, &other).expect("reject foreign transcript")
        );
    }
}
