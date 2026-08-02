use std::fs;
use std::process::Command;

use brain_coordination::{PreflightBlocker, merge_preflight};

#[test]
fn preflight_finds_text_conflict_without_changing_branch_index_or_worktree() {
    let repo = conflict_repo();
    let before = snapshot(&repo.path);
    let result = merge_preflight(&repo.path, "HEAD", "target").expect("preflight");

    assert!(
        result
            .conflicts
            .contains(&std::path::PathBuf::from("src/auth.rs")),
        "{result:?}"
    );
    assert!(!result.ready);
    assert_eq!(snapshot(&repo.path), before);
}

#[test]
fn dirty_worktree_is_reported_and_never_stashed_or_cleaned() {
    let repo = conflict_repo();
    fs::write(repo.path.join("local edit.txt"), "uncommitted\n").expect("local edit");
    let result = merge_preflight(&repo.path, "HEAD", "target").expect("preflight");
    assert!(result.blockers.contains(&PreflightBlocker::DirtyWorktree));
    assert!(repo.path.join("local edit.txt").is_file());
}

#[test]
fn disjoint_changes_are_ready_without_heuristic_conflict_claims() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("clean repo");
    init(&path);
    git(&path, &["branch", "target"]);
    fs::write(path.join("source.txt"), "source\n").expect("source");
    git(&path, &["add", "source.txt"]);
    git(&path, &["commit", "-m", "source"]);
    git(&path, &["checkout", "target"]);
    fs::write(path.join("target.txt"), "target\n").expect("target");
    git(&path, &["add", "target.txt"]);
    git(&path, &["commit", "-m", "target"]);
    git(&path, &["checkout", "-"]);
    let result = merge_preflight(&path, "HEAD", "target").expect("preflight");
    assert!(result.ready, "{result:?}");
    assert!(result.conflicts.is_empty());
}

struct Repo {
    _temp: tempfile::TempDir,
    path: std::path::PathBuf,
}

fn conflict_repo() -> Repo {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("conflict repo");
    init(&path);
    git(&path, &["branch", "target"]);
    fs::write(path.join("src").join("auth.rs"), "source change\n").expect("source change");
    git(&path, &["add", "src/auth.rs"]);
    git(&path, &["commit", "-m", "source change"]);
    git(&path, &["checkout", "target"]);
    fs::write(path.join("src").join("auth.rs"), "target change\n").expect("target change");
    git(&path, &["add", "src/auth.rs"]);
    git(&path, &["commit", "-m", "target change"]);
    git(&path, &["checkout", "-"]);
    Repo { _temp: temp, path }
}

fn init(path: &std::path::Path) {
    fs::create_dir_all(path.join("src")).expect("repo");
    git(path, &["init"]);
    git(path, &["config", "user.email", "fixture@example.invalid"]);
    git(path, &["config", "user.name", "Fixture"]);
    fs::write(path.join("src").join("auth.rs"), "base\n").expect("base");
    git(path, &["add", "."]);
    git(path, &["commit", "-m", "base"]);
}

fn snapshot(path: &std::path::Path) -> (String, String, String, String) {
    (
        output(path, &["rev-parse", "HEAD"]),
        output(path, &["branch", "--show-current"]),
        output(path, &["diff", "--cached"]),
        output(path, &["status", "--porcelain"]),
    )
}

fn git(path: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
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

fn output(path: &std::path::Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {}", arguments.join(" "));
    String::from_utf8(output.stdout).expect("UTF-8")
}
