use std::fs;
use std::process::Command;

use brain_coordination::TaskWorktreeManager;
use brain_domain::ProjectRegistry;

#[test]
fn task_create_reuses_project_identity_and_gets_a_unique_validated_worktree() {
    let fixture = RepoFixture::new();
    let manager = fixture.manager();
    let task = manager
        .create("OAuth callback", None, fixture.now)
        .expect("create task worktree");

    assert_eq!(task.project_id, fixture.project_id);
    assert_ne!(task.worktree_id, fixture.main_worktree_id);
    let approved_parent = fs::canonicalize(&fixture.worktree_parent).expect("approved parent");
    assert!(
        task.worktree_path
            .as_deref()
            .expect("task path")
            .starts_with(approved_parent)
    );
    assert!(task.worktree_path.as_deref().expect("path").is_dir());
    let worktrees = manager.list_git_worktrees().expect("list worktrees");
    assert!(
        worktrees
            .iter()
            .any(|worktree| Some(&worktree.path) == task.worktree_path.as_ref())
    );
}

#[test]
fn close_reports_dirty_state_and_never_removes_the_worktree() {
    let fixture = RepoFixture::new();
    let manager = fixture.manager();
    let task = manager
        .create("Dirty task", None, fixture.now)
        .expect("create task");
    let path = task.worktree_path.clone().expect("task path");
    fs::write(path.join("dirty.txt"), "local edit\n").expect("dirty file");

    let report = manager
        .close(task.id, fixture.now + time::Duration::hours(1))
        .expect("close task");
    assert!(report.dirty);
    assert!(path.is_dir(), "close must not remove the worktree");
    assert!(
        report
            .cleanup_command
            .expect("cleanup guidance")
            .contains("worktree remove")
    );
}

struct RepoFixture {
    _temp: tempfile::TempDir,
    brain_home: std::path::PathBuf,
    repo: std::path::PathBuf,
    ledger: std::path::PathBuf,
    worktree_parent: std::path::PathBuf,
    project_id: brain_domain::ProjectId,
    main_worktree_id: brain_domain::WorktreeId,
    now: time::OffsetDateTime,
}

impl RepoFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let brain_home = temp.path().join("brain");
        let repo = temp.path().join("repo with spaces");
        let worktree_parent = temp.path().join("agent worktrees");
        fs::create_dir_all(&repo).expect("repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        git(&repo, &["config", "user.name", "Fixture"]);
        fs::write(repo.join("README.md"), "fixture\n").expect("readme");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "initial"]);
        let identity = ProjectRegistry::open(&brain_home)
            .expect("registry")
            .register(&repo)
            .expect("register");
        let ledger = brain_home
            .join("projects")
            .join(identity.project_id.0.to_string())
            .join("ledger.sqlite");
        Self {
            _temp: temp,
            brain_home,
            repo,
            ledger,
            worktree_parent,
            project_id: identity.project_id,
            main_worktree_id: identity.worktree_id,
            now: time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(20_000),
        }
    }

    fn manager(&self) -> TaskWorktreeManager {
        TaskWorktreeManager::new(
            &self.brain_home,
            self.project_id,
            &self.repo,
            &self.ledger,
            &self.worktree_parent,
        )
        .expect("manager")
    }
}

fn git(root: &std::path::Path, arguments: &[&str]) {
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
