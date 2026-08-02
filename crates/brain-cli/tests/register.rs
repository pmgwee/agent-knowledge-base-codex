use brain_cli::{RegisterOptions, register_project};
use brain_service::ServiceLaunchConfig;

#[test]
fn registration_creates_stable_project_storage_and_service_config() {
    let temp = tempfile::tempdir().expect("create registration fixture");
    let brain_home = temp.path().join("brain");
    let project = temp.path().join("project");
    let transcript = temp.path().join("claude").join("session.jsonl");
    std::fs::create_dir_all(&project).expect("create project");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("create transcript directory");
    std::fs::write(&transcript, "").expect("create transcript");

    let first = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project.clone(),
        claude_projects_root: None,
        explicit_claude_sources: vec![transcript.clone()],
        pipe_name: Some(r"\\.\pipe\registration-fixture".to_owned()),
    })
    .expect("register project");
    let second = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project,
        claude_projects_root: None,
        explicit_claude_sources: vec![transcript.clone()],
        pipe_name: Some(r"\\.\pipe\registration-fixture".to_owned()),
    })
    .expect("repeat registration");

    assert_eq!(first.project_id, second.project_id);
    assert_eq!(first.worktree_id, second.worktree_id);
    assert_eq!(first.ledger_path, second.ledger_path);
    assert!(first.ledger_path.is_file());
    assert!(brain_home.join("projects.json").is_file());

    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))
        .expect("load generated service configuration");
    let project = config
        .project(Some(first.project_id))
        .expect("select project");
    assert_eq!(project.project_id, first.project_id);
    assert_eq!(project.worktree_id, first.worktree_id);
    assert_eq!(project.claude_sources, vec![transcript]);
    assert_eq!(config.pipe_name, r"\\.\pipe\registration-fixture");
}

#[test]
fn registering_a_second_project_preserves_the_first_service_scope() {
    let temp = tempfile::tempdir().expect("create multi-project fixture");
    let brain_home = temp.path().join("brain");
    let project_a = temp.path().join("project-a");
    let project_b = temp.path().join("project-b");
    std::fs::create_dir_all(&project_a).expect("create project A");
    std::fs::create_dir_all(&project_b).expect("create project B");

    let first = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project_a,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: Some(r"\\.\pipe\multi-project-fixture".to_owned()),
    })
    .expect("register A");
    let second = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project_b,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: None,
    })
    .expect("register B");

    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))
        .expect("load multi-project service config");
    assert_eq!(config.projects.len(), 2);
    assert!(config.project(Some(first.project_id)).is_ok());
    assert!(config.project(Some(second.project_id)).is_ok());
    assert_ne!(first.ledger_path, second.ledger_path);
    assert_eq!(config.pipe_name, r"\\.\pipe\multi-project-fixture");
}
