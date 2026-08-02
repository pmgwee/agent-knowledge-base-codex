use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use brain_domain::WorktreeId;

const GIT_STATUS_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_DIRTY_PATHS: usize = 100;

#[derive(Clone, Debug)]
pub struct LiveState {
    pub worktree_id: WorktreeId,
    pub available: bool,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub dirty: bool,
    pub dirty_paths: Vec<String>,
    pub conflicts: Vec<String>,
    pub submodules_dirty: bool,
    pub observed_at: time::OffsetDateTime,
    pub timed_out: bool,
    pub error: Option<String>,
}

impl LiveState {
    pub fn inspect(root: &Path, worktree_id: WorktreeId) -> Self {
        let observed_at = time::OffsetDateTime::now_utc();
        match git_status(root) {
            Ok(output) => parse_status(worktree_id, observed_at, &output),
            Err(GitStatusError::Timeout) => {
                Self::unavailable(worktree_id, observed_at, true, "git status timed out")
            }
            Err(GitStatusError::Unavailable(reason)) => {
                Self::unavailable(worktree_id, observed_at, false, &reason)
            }
        }
    }

    pub fn fixture_clean(worktree_id: WorktreeId, branch: &str, head: &str) -> Self {
        Self {
            worktree_id,
            available: true,
            head: Some(head.to_owned()),
            branch: Some(branch.to_owned()),
            upstream: None,
            ahead: 0,
            behind: 0,
            dirty: false,
            dirty_paths: Vec::new(),
            conflicts: Vec::new(),
            submodules_dirty: false,
            observed_at: time::OffsetDateTime::now_utc(),
            timed_out: false,
            error: None,
        }
    }

    pub fn compatible_revision(&self, captured_head: Option<&str>) -> bool {
        self.available && self.head.as_deref().is_some() && self.head.as_deref() == captured_head
    }

    fn unavailable(
        worktree_id: WorktreeId,
        observed_at: time::OffsetDateTime,
        timed_out: bool,
        reason: &str,
    ) -> Self {
        Self {
            worktree_id,
            available: false,
            head: None,
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            dirty: false,
            dirty_paths: Vec::new(),
            conflicts: Vec::new(),
            submodules_dirty: false,
            observed_at,
            timed_out,
            error: Some(reason.to_owned()),
        }
    }
}

enum GitStatusError {
    Timeout,
    Unavailable(String),
}

fn git_status(root: &Path) -> Result<String, GitStatusError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| GitStatusError::Unavailable(error.to_string()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < GIT_STATUS_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitStatusError::Timeout);
            }
            Err(error) => return Err(GitStatusError::Unavailable(error.to_string())),
        }
    };
    if !status.success() {
        return Err(GitStatusError::Unavailable(
            "worktree is not available to git status".to_owned(),
        ));
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or_else(|| GitStatusError::Unavailable("git stdout is unavailable".to_owned()))?
        .read_to_string(&mut output)
        .map_err(|error| GitStatusError::Unavailable(error.to_string()))?;
    Ok(output)
}

fn parse_status(
    worktree_id: WorktreeId,
    observed_at: time::OffsetDateTime,
    output: &str,
) -> LiveState {
    let mut state = LiveState {
        worktree_id,
        available: true,
        head: None,
        branch: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty: false,
        dirty_paths: Vec::new(),
        conflicts: Vec::new(),
        submodules_dirty: false,
        observed_at,
        timed_out: false,
        error: None,
    };
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("# branch.oid ") {
            state.head = (value != "(initial)").then(|| value.to_owned());
        } else if let Some(value) = line.strip_prefix("# branch.head ") {
            state.branch = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("# branch.upstream ") {
            state.upstream = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("# branch.ab ") {
            for item in value.split_whitespace() {
                if let Some(ahead) = item.strip_prefix('+') {
                    state.ahead = ahead.parse().unwrap_or(0);
                } else if let Some(behind) = item.strip_prefix('-') {
                    state.behind = behind.parse().unwrap_or(0);
                }
            }
        } else if let Some(path) = line.strip_prefix("? ") {
            push_dirty(&mut state, path);
        } else if line.starts_with("u ") {
            let path = line.splitn(11, ' ').nth(10).unwrap_or("unknown");
            if state.conflicts.len() < MAX_DIRTY_PATHS {
                state.conflicts.push(path.to_owned());
            }
            push_dirty(&mut state, path);
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.get(2).is_some_and(|value| value.starts_with('S')) {
                state.submodules_dirty = true;
            }
            let path = if line.starts_with("1 ") {
                line.splitn(9, ' ').nth(8)
            } else {
                line.splitn(10, ' ')
                    .nth(9)
                    .and_then(|paths| paths.split('\t').next())
            };
            if let Some(path) = path {
                push_dirty(&mut state, path);
            }
        }
    }
    state.dirty |= !state.conflicts.is_empty() || state.submodules_dirty;
    state
}

fn push_dirty(state: &mut LiveState, path: &str) {
    state.dirty = true;
    if state.dirty_paths.len() < MAX_DIRTY_PATHS
        && !state.dirty_paths.iter().any(|existing| existing == path)
    {
        state.dirty_paths.push(path.to_owned());
    }
}
