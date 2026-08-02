use brain_domain::ProjectIdentity;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn equal_basenames_do_not_collide() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let first = temp.path().join("one").join("api");
    let second = temp.path().join("two").join("api");
    std::fs::create_dir_all(&first).expect("create first project");
    std::fs::create_dir_all(&second).expect("create second project");

    let first_identity = ProjectIdentity::for_non_git(&first).expect("inspect first project");
    let second_identity = ProjectIdentity::for_non_git(&second).expect("inspect second project");

    assert_ne!(first_identity.project_key, second_identity.project_key);
}

#[test]
fn linked_worktrees_share_a_project_key_but_not_a_worktree_key() {
    let repo = TestRepo::with_linked_worktree();

    let main = ProjectIdentity::inspect(&repo.main).expect("inspect main worktree");
    let linked = ProjectIdentity::inspect(&repo.linked).expect("inspect linked worktree");

    assert_eq!(main.project_key, linked.project_key);
    assert_ne!(main.worktree_key, linked.worktree_key);
}

#[test]
fn inspected_identity_round_trips_for_service_boundaries() {
    let temp = tempfile::tempdir().expect("create project fixture");
    let identity = ProjectIdentity::for_non_git(temp.path()).expect("inspect project");

    let json = serde_json::to_string(&identity).expect("serialize project identity");
    let decoded: ProjectIdentity =
        serde_json::from_str(&json).expect("deserialize project identity");

    assert_eq!(decoded, identity);
}

struct TestRepo {
    _temp: tempfile::TempDir,
    main: PathBuf,
    linked: PathBuf,
}

impl TestRepo {
    fn with_linked_worktree() -> Self {
        let temp = tempfile::tempdir().expect("create repository fixture root");
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        std::fs::create_dir_all(&main).expect("create main worktree");

        git(&main, &["init", "-b", "main"]);
        std::fs::write(main.join("README.md"), "fixture\n").expect("write fixture file");
        git(&main, &["add", "README.md"]);
        git(
            &main,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                linked.to_str().expect("UTF-8 fixture path"),
                "-b",
                "feature",
            ],
        );

        Self {
            _temp: temp,
            main,
            linked,
        }
    }
}

fn git(cwd: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(cwd)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
