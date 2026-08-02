use std::process::Command;

use brain_context::LiveState;
use brain_domain::WorktreeId;

#[test]
fn git_live_state_reports_head_branch_and_dirty_unicode_paths() {
    let temp = tempfile::tempdir().expect("create git fixture");
    run(temp.path(), ["init"]);
    run(temp.path(), ["config", "user.email", "fixture@example.com"]);
    run(temp.path(), ["config", "user.name", "Fixture"]);
    std::fs::write(temp.path().join("tracked.txt"), "initial\n").expect("write tracked file");
    run(temp.path(), ["add", "tracked.txt"]);
    run(temp.path(), ["commit", "-m", "initial"]);
    std::fs::write(temp.path().join("tracked.txt"), "changed\n").expect("modify tracked file");
    std::fs::write(temp.path().join("unicode \u{8bb0}\u{5fc6}.txt"), "new\n")
        .expect("write Unicode file");

    let state = LiveState::inspect(temp.path(), WorktreeId(uuid::Uuid::now_v7()));
    assert!(state.available, "{:?}", state.error);
    assert!(state.head.is_some());
    assert!(state.branch.is_some());
    assert!(state.dirty);
    assert!(
        state
            .dirty_paths
            .iter()
            .any(|path| path.contains("tracked.txt"))
    );
    assert!(
        state
            .dirty_paths
            .iter()
            .any(|path| path.contains("unicode \u{8bb0}\u{5fc6}.txt"))
    );
}

fn run<const N: usize>(root: &std::path::Path, args: [&str; N]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("run git fixture command");
    assert!(status.success());
}
