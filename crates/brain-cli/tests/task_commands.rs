use std::fs;
use std::process::Command;

use brain_cli::TaskCommands;
use brain_domain::ProjectRegistry;
use brain_service::{ServiceLaunchConfig, ServiceProjectConfig};

#[test]
fn task_commands_create_list_and_close_without_deleting_the_worktree() {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let repo = temp.path().join("repo");
    let worktree_parent = temp.path().join("task worktrees");
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
    let ledger_path = brain_home
        .join("projects")
        .join(identity.project_id.0.to_string())
        .join("ledger.sqlite");
    let mut config = ServiceLaunchConfig::new(r"\\.\pipe\task-cli-fixture");
    config.upsert_project(ServiceProjectConfig {
        project_root: identity.root,
        project_id: identity.project_id,
        worktree_id: identity.worktree_id,
        ledger_path,
        claude_sources: Vec::new(),
        codex_sources: Vec::new(),
        hermes_database: None,
    });
    let config_path = ServiceLaunchConfig::default_path(&brain_home);
    fs::create_dir_all(config_path.parent().expect("runtime parent")).expect("runtime");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).expect("config JSON"),
    )
    .expect("write config");

    let commands = TaskCommands::open(
        &brain_home,
        &identity.project_id.0.to_string(),
        Some(worktree_parent),
    )
    .expect("commands");
    let task = commands
        .create("OAuth callback", None)
        .expect("create task");
    let path = task.worktree_path.clone().expect("path");
    assert_eq!(commands.list(false).expect("list"), vec![task.clone()]);
    let closed = commands.close(task.id).expect("close");
    assert!(path.is_dir());
    assert!(closed.cleanup_command.is_some());
    assert!(commands.list(false).expect("active list").is_empty());
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
