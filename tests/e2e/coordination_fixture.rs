use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use brain_coordination::{CoordinationStore, TaskRecord, TaskWorktreeManager};
use brain_domain::{Harness, ProjectId, ProjectRegistry};

pub struct CoordinationE2e {
    _temp: tempfile::TempDir,
    pub brain_home: PathBuf,
    pub project_root: PathBuf,
    pub ledger_path: PathBuf,
    pub worktree_parent: PathBuf,
    pub project_id: ProjectId,
    pub now: time::OffsetDateTime,
}

impl CoordinationE2e {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("create coordination fixture");
        let brain_home = temp.path().join("brain");
        let project_root = temp.path().join("project with spaces");
        let worktree_parent = temp.path().join("task worktrees");
        fs::create_dir_all(project_root.join("src/auth")).expect("create project");
        fs::create_dir_all(&worktree_parent).expect("create worktree parent");
        let worktree_parent =
            fs::canonicalize(worktree_parent).expect("canonicalize worktree parent");
        git(&project_root, &["init"]);
        git(
            &project_root,
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(&project_root, &["config", "user.name", "Fixture"]);
        fs::write(
            project_root.join("src/auth/callback.rs"),
            "fn callback() {}\n",
        )
        .expect("write source");
        fs::write(project_root.join("README.md"), "fixture\n").expect("write README");
        git(&project_root, &["add", "."]);
        git(&project_root, &["commit", "-m", "initial"]);
        let identity = ProjectRegistry::open(&brain_home)
            .expect("open registry")
            .register(&project_root)
            .expect("register project");
        let ledger_path = brain_home
            .join("projects")
            .join(identity.project_id.0.to_string())
            .join("ledger.sqlite");
        Self {
            _temp: temp,
            brain_home,
            project_root,
            ledger_path,
            worktree_parent,
            project_id: identity.project_id,
            now: time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000),
        }
    }

    pub fn start_task(&self, title: &str) -> TaskRecord {
        self.manager()
            .create(title, None, self.now)
            .expect("create task worktree")
    }

    pub fn manager(&self) -> TaskWorktreeManager {
        TaskWorktreeManager::new(
            &self.brain_home,
            self.project_id,
            &self.project_root,
            &self.ledger_path,
            &self.worktree_parent,
        )
        .expect("create worktree manager")
    }

    pub fn store(&self) -> CoordinationStore {
        CoordinationStore::open(&self.ledger_path, self.project_id)
            .expect("open coordination store")
    }
}

pub fn session(harness: Harness, id: &str) -> brain_coordination::SessionIdentity {
    brain_coordination::SessionIdentity {
        harness,
        native_session_id: id.to_owned(),
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {}: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
