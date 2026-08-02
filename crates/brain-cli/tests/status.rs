use brain_cli::{RegisterOptions, read_status, register_project};

#[test]
fn status_reports_registered_scope_and_empty_ledger() {
    let temp = tempfile::tempdir().expect("create status fixture");
    let brain_home = temp.path().join("brain");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let registered = register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: None,
    })
    .expect("register project");

    let status = read_status(&brain_home, Some(registered.project_id)).expect("read status");

    assert_eq!(status.project_id, registered.project_id);
    assert_eq!(status.worktree_id, registered.worktree_id);
    assert_eq!(status.persisted_events, 0);
    assert_eq!(status.source_count, 0);
    assert!(status.last_event_at.is_none());
    assert!(status.ledger_path.is_file());
}
