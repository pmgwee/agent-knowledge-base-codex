//! Whether the installed brain matches the source it was built from.
//!
//! The binaries the system actually runs live in `<brain-home>/bin`, copied there by a deploy
//! step. Nothing about that copy is automatic from the compiler's point of view: a source
//! change that is built but never copied leaves every reader — the dashboard included — talking
//! to an older binary while every field it renders still looks entirely plausible. That is the
//! same silent-divergence failure as a registration snapshot that stops growing, and it needs
//! the same treatment: make the divergence a value someone can see.
//!
//! Three questions are answered here, each independently:
//!
//! - **Is the source ahead of what was deployed?** Compared by commit, read straight from the
//!   git directory rather than by shelling out, so a dashboard poll stays a pure file read.
//! - **Did the last deploy succeed?** A failed build must leave the previous binaries in place,
//!   so "running fine" and "last deploy failed" are both true at once and both worth showing.
//! - **Are the installed binaries the ones that deploy wrote?** Verified by hashing what is on
//!   disk against the hashes recorded at deploy time, which catches a half-finished copy or a
//!   file that could not be replaced because it was locked by a running process.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Manifest file written by the deploy script, relative to `<brain-home>/runtime`.
pub const DEPLOY_MANIFEST: &str = "deploy.json";

/// The binaries a deploy is responsible for replacing.
///
/// `brain-mcp` is included even though only Codex launches it: leaving it out is precisely how
/// it drifted a full session behind the others before this existed.
pub const DEPLOYED_BINARIES: [&str; 4] = [
    "brain.exe",
    "brain-service.exe",
    "brain-hook.exe",
    "brain-mcp.exe",
];

/// Outcome of the most recent deploy attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeployStatus {
    /// A deploy is in flight. The manifest is written before the build starts so a crash
    /// mid-build is distinguishable from never having deployed.
    Running,
    /// Build, copy, and restart all completed.
    Succeeded,
    /// The build failed, so nothing was replaced. The previous deployment is still live.
    Failed,
}

impl DeployStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// One binary as recorded at deploy time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestBinary {
    pub name: String,
    /// Absent when the deploy could not read the built artifact.
    #[serde(default)]
    pub sha256: Option<String>,
    /// False when the file could not be overwritten — typically because a process still held
    /// it open. The old binary is then still in place and the deploy is not fully applied.
    #[serde(default)]
    pub replaced: bool,
}

/// The deploy script's record of what it did. Every field past `schema_version` is optional so
/// that a manifest written by an older script, or truncated by a crash, still parses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployManifest {
    pub schema_version: u32,
    pub status: DeployStatus,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub source_root: Option<PathBuf>,
    /// The working tree had uncommitted changes when this was built, so the binaries contain
    /// code that is in no commit and `commit` only approximates what is installed.
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub started_at: Option<time::OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<time::OffsetDateTime>,
    #[serde(default)]
    pub binaries: Vec<ManifestBinary>,
    #[serde(default)]
    pub service_restarted: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub log: Option<PathBuf>,
}

/// One binary as it exists on disk right now, checked against the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct DeployedBinary {
    pub name: String,
    pub present: bool,
    /// Whether the file on disk hashes to what the deploy recorded. False here is the signal
    /// that something replaced or failed to replace this binary outside the deploy path.
    pub matches_manifest: bool,
    pub sha256: Option<String>,
}

/// Deployment state as the dashboard renders it.
#[derive(Debug, Clone, Serialize)]
pub struct DeploymentDashboard {
    /// False when no deploy has ever run, which is also how a fresh install looks. Every other
    /// field is then meaningless and the dashboard should say "not configured" rather than
    /// implying anything is wrong.
    pub configured: bool,
    pub status: String,
    pub deployed_commit: Option<String>,
    pub deployed_branch: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub deployed_at: Option<time::OffsetDateTime>,
    /// Current commit of the source tree the last deploy was built from.
    pub head_commit: Option<String>,
    /// The source tree has moved on since the last successful deploy.
    pub source_is_ahead: bool,
    /// Built from a working tree with uncommitted changes.
    ///
    /// Deliberately kept out of `up_to_date`: such a deploy really did install what the tree
    /// held at that moment, so calling it stale would be wrong. But the commit no longer
    /// identifies what is running, and later edits to those same files cannot be detected by
    /// comparing commits — so the caveat is worth showing on its own.
    pub deployed_dirty: bool,
    pub binaries: Vec<DeployedBinary>,
    /// Binaries whose on-disk content is not what the deploy recorded.
    pub drifted_binaries: Vec<String>,
    /// Binaries the deploy could not replace, usually because a process held them open.
    pub unreplaced_binaries: Vec<String>,
    pub error: Option<String>,
    /// The single bit worth alerting on: the installed brain is the source tree, fully applied.
    pub up_to_date: bool,
}

impl DeploymentDashboard {
    /// State for a brain that has never been deployed. Reported as "unknown" rather than
    /// "stale", because claiming drift we have not measured would be its own false signal.
    fn unconfigured() -> Self {
        Self {
            configured: false,
            status: "never".to_string(),
            deployed_commit: None,
            deployed_branch: None,
            deployed_at: None,
            head_commit: None,
            source_is_ahead: false,
            deployed_dirty: false,
            binaries: Vec::new(),
            drifted_binaries: Vec::new(),
            unreplaced_binaries: Vec::new(),
            error: None,
            up_to_date: false,
        }
    }
}

/// Read deployment state for a brain home.
///
/// Never fails: a missing, unreadable, or malformed manifest reports "not configured" rather
/// than taking down the whole snapshot. Deployment state is diagnostic, and a dashboard that
/// refuses to render because it could not answer one diagnostic question is worse than one
/// that renders and says it does not know.
pub fn read_deployment(brain_home: &Path) -> DeploymentDashboard {
    let path = brain_home.join("runtime").join(DEPLOY_MANIFEST);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return DeploymentDashboard::unconfigured();
    };
    let Ok(manifest) = serde_json::from_str::<DeployManifest>(&text) else {
        return DeploymentDashboard::unconfigured();
    };

    let bin_dir = brain_home.join("bin");
    let binaries = verify_binaries(&bin_dir, &manifest);
    let drifted: Vec<String> = binaries
        .iter()
        .filter(|binary| !binary.matches_manifest)
        .map(|binary| binary.name.clone())
        .collect();
    let unreplaced: Vec<String> = manifest
        .binaries
        .iter()
        .filter(|binary| !binary.replaced)
        .map(|binary| binary.name.clone())
        .collect();

    let head_commit = manifest.source_root.as_deref().and_then(read_head_commit);
    // Only claim the source moved on when both commits are known. An unreadable git directory
    // means we cannot tell, and "cannot tell" must not render as "behind".
    let source_is_ahead = match (&head_commit, &manifest.commit) {
        (Some(head), Some(deployed)) => head != deployed,
        _ => false,
    };

    let up_to_date = manifest.status == DeployStatus::Succeeded
        && !source_is_ahead
        && drifted.is_empty()
        && unreplaced.is_empty();

    DeploymentDashboard {
        configured: true,
        status: manifest.status.as_str().to_string(),
        deployed_commit: manifest.commit,
        deployed_branch: manifest.branch,
        deployed_at: manifest.finished_at.or(manifest.started_at),
        head_commit,
        source_is_ahead,
        deployed_dirty: manifest.dirty,
        binaries,
        drifted_binaries: drifted,
        unreplaced_binaries: unreplaced,
        error: manifest.error,
        up_to_date,
    }
}

/// Hash every installed binary and compare against what the deploy recorded.
///
/// A binary the manifest does not mention is reported as present-and-matching: it is not
/// evidence of drift, only of a manifest written before that binary was part of the deploy set.
fn verify_binaries(bin_dir: &Path, manifest: &DeployManifest) -> Vec<DeployedBinary> {
    DEPLOYED_BINARIES
        .iter()
        .map(|name| {
            let path = bin_dir.join(name);
            let actual = file_sha256(&path);
            let expected = manifest
                .binaries
                .iter()
                .find(|binary| binary.name.eq_ignore_ascii_case(name))
                .and_then(|binary| binary.sha256.as_deref());
            let matches = match (&actual, expected) {
                (Some(actual), Some(expected)) => actual.eq_ignore_ascii_case(expected),
                // Unrecorded binary, or one we could not read: not evidence of drift.
                (_, None) => true,
                (None, Some(_)) => false,
            };
            DeployedBinary {
                name: (*name).to_string(),
                present: path.is_file(),
                matches_manifest: matches,
                sha256: actual,
            }
        })
        .collect()
}

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(hex::encode(hasher.finalize()))
}

/// The commit a source tree currently points at, read directly from the git directory.
///
/// Reading the files rather than invoking git keeps this usable from a dashboard poll: no
/// subprocess, no PATH assumption, no failure mode where a missing git binary reports the
/// source as diverged. Any shape this does not understand returns `None`, which the caller
/// treats as "cannot tell" rather than as drift.
pub fn read_head_commit(source_root: &Path) -> Option<String> {
    let git_dir = resolve_git_dir(source_root)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    let Some(reference) = head.strip_prefix("ref: ") else {
        // Detached HEAD stores the object id itself.
        return is_object_id(head).then(|| head.to_string());
    };
    let reference = reference.trim();

    // A loose ref is a file holding the id; once packed, it moves into `packed-refs`.
    if let Ok(loose) = std::fs::read_to_string(git_dir.join(reference)) {
        let loose = loose.trim();
        if is_object_id(loose) {
            return Some(loose.to_string());
        }
    }
    let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (id, name) = line.split_once(' ')?;
        (name.trim() == reference && is_object_id(id)).then(|| id.to_string())
    })
}

/// Locate the git directory for a working tree.
///
/// Usually `.git/`, but a worktree or submodule stores a `gitdir:` pointer file instead, and
/// the brain is plausible to develop from a worktree.
fn resolve_git_dir(source_root: &Path) -> Option<PathBuf> {
    let candidate = source_root.join(".git");
    if candidate.is_dir() {
        return Some(candidate);
    }
    let pointer = std::fs::read_to_string(&candidate).ok()?;
    let target = PathBuf::from(pointer.trim().strip_prefix("gitdir: ")?.trim());
    Some(if target.is_absolute() {
        target
    } else {
        source_root.join(target)
    })
}

/// Both SHA-1 and SHA-256 repository formats, so this does not silently stop working if the
/// repository is ever converted.
fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn a_brain_that_was_never_deployed_reports_unknown_not_stale() {
        let temp = tempfile::tempdir().expect("temp");
        let state = read_deployment(temp.path());
        assert!(!state.configured);
        assert_eq!(state.status, "never");
        assert!(
            !state.source_is_ahead,
            "absence of a manifest is not evidence of drift"
        );
    }

    #[test]
    fn head_commit_is_read_from_a_loose_ref() {
        let temp = tempfile::tempdir().expect("temp");
        let id = "a".repeat(40);
        write(
            &temp.path().join(".git").join("HEAD"),
            "ref: refs/heads/main\n",
        );
        write(
            &temp
                .path()
                .join(".git")
                .join("refs")
                .join("heads")
                .join("main"),
            &format!("{id}\n"),
        );
        assert_eq!(read_head_commit(temp.path()), Some(id));
    }

    #[test]
    fn head_commit_falls_back_to_packed_refs() {
        // Git packs refs on `gc`, at which point the loose file disappears. Without this
        // fallback the dashboard would report "cannot tell" on any well-maintained repository.
        let temp = tempfile::tempdir().expect("temp");
        let id = "b".repeat(40);
        write(
            &temp.path().join(".git").join("HEAD"),
            "ref: refs/heads/main\n",
        );
        write(
            &temp.path().join(".git").join("packed-refs"),
            &format!("# pack-refs with: peeled fully-peeled sorted\n{id} refs/heads/main\n"),
        );
        assert_eq!(read_head_commit(temp.path()), Some(id));
    }

    #[test]
    fn a_detached_head_reports_its_object_id() {
        let temp = tempfile::tempdir().expect("temp");
        let id = "c".repeat(40);
        write(&temp.path().join(".git").join("HEAD"), &format!("{id}\n"));
        assert_eq!(read_head_commit(temp.path()), Some(id));
    }

    #[test]
    fn a_source_tree_ahead_of_the_deploy_is_not_up_to_date() {
        // The defect this guards: source changed, deploy never ran, and every dashboard field
        // still renders plausibly against the older binary.
        let temp = tempfile::tempdir().expect("temp");
        let source = temp.path().join("source");
        let brain_home = temp.path().join("brain");
        write(&source.join(".git").join("HEAD"), "ref: refs/heads/main\n");
        write(
            &source.join(".git").join("refs").join("heads").join("main"),
            &format!("{}\n", "d".repeat(40)),
        );

        let manifest = serde_json::json!({
            "schema_version": 1,
            "status": "succeeded",
            "commit": "e".repeat(40),
            "branch": "main",
            "source_root": source,
            "started_at": "2026-08-05T12:00:00Z",
            "finished_at": "2026-08-05T12:01:00Z",
            "binaries": [],
            "service_restarted": true,
        });
        write(
            &brain_home.join("runtime").join(DEPLOY_MANIFEST),
            &manifest.to_string(),
        );

        let state = read_deployment(&brain_home);
        assert!(state.configured);
        assert!(
            state.source_is_ahead,
            "a newer commit must register as ahead"
        );
        assert!(!state.up_to_date);
    }

    #[test]
    fn a_binary_replaced_outside_the_deploy_registers_as_drift() {
        // Catches a partial copy, a hand-placed build, or a file that quietly failed to write.
        let temp = tempfile::tempdir().expect("temp");
        let brain_home = temp.path().join("brain");
        std::fs::create_dir_all(brain_home.join("bin")).expect("bin");
        std::fs::write(brain_home.join("bin").join("brain.exe"), b"actual bytes").expect("write");

        let manifest = serde_json::json!({
            "schema_version": 1,
            "status": "succeeded",
            "binaries": [{ "name": "brain.exe", "sha256": "f".repeat(64), "replaced": true }],
        });
        write(
            &brain_home.join("runtime").join(DEPLOY_MANIFEST),
            &manifest.to_string(),
        );

        let state = read_deployment(&brain_home);
        assert_eq!(state.drifted_binaries, vec!["brain.exe".to_string()]);
        assert!(!state.up_to_date);
    }

    #[test]
    fn a_failed_build_reports_failure_while_the_old_binaries_stay_valid() {
        // Fail-safe deploy: a broken commit must leave the previous deployment live, so
        // "running fine" and "last deploy failed" are both true and both worth showing.
        let temp = tempfile::tempdir().expect("temp");
        let brain_home = temp.path().join("brain");
        let manifest = serde_json::json!({
            "schema_version": 1,
            "status": "failed",
            "error": "error[E0308]: mismatched types",
            "binaries": [],
        });
        write(
            &brain_home.join("runtime").join(DEPLOY_MANIFEST),
            &manifest.to_string(),
        );

        let state = read_deployment(&brain_home);
        assert_eq!(state.status, "failed");
        assert!(!state.up_to_date);
        assert!(state.error.is_some_and(|error| error.contains("E0308")));
    }

    #[test]
    fn a_deploy_from_a_dirty_tree_is_flagged_without_being_called_stale() {
        // Such a deploy really did install what the tree held, so reporting it as behind would
        // be wrong. But the commit no longer identifies what is running, and that caveat has
        // to reach the surface rather than being rounded off to "up to date".
        let temp = tempfile::tempdir().expect("temp");
        let brain_home = temp.path().join("brain");
        let manifest = serde_json::json!({
            "schema_version": 1,
            "status": "succeeded",
            "commit": "a".repeat(40),
            "dirty": true,
            "binaries": [],
        });
        write(
            &brain_home.join("runtime").join(DEPLOY_MANIFEST),
            &manifest.to_string(),
        );

        let state = read_deployment(&brain_home);
        assert!(state.deployed_dirty);
        assert!(
            state.up_to_date,
            "a dirty deploy installed what the tree held; it is not stale"
        );
    }

    #[test]
    fn a_binary_that_could_not_be_replaced_is_surfaced() {
        // brain-mcp.exe is held open whenever Codex is running, so this is the ordinary case,
        // not an exotic one. A deploy that silently skipped it would leave Codex on old code.
        let temp = tempfile::tempdir().expect("temp");
        let brain_home = temp.path().join("brain");
        let manifest = serde_json::json!({
            "schema_version": 1,
            "status": "succeeded",
            "binaries": [{ "name": "brain-mcp.exe", "sha256": null, "replaced": false }],
        });
        write(
            &brain_home.join("runtime").join(DEPLOY_MANIFEST),
            &manifest.to_string(),
        );

        let state = read_deployment(&brain_home);
        assert_eq!(state.unreplaced_binaries, vec!["brain-mcp.exe".to_string()]);
        assert!(!state.up_to_date);
    }
}
