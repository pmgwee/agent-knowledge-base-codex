use brain_cli::{RegisterOptions, read_hermes_status, register_project};

#[test]
fn hermes_status_activates_only_the_reviewed_project_bound_schema() {
    let temp = tempfile::tempdir().expect("create Hermes status fixture");
    let brain_home = temp.path().join("brain");
    let project = temp.path().join("project");
    let reviewed_db = temp.path().join("reviewed.db");
    let drifted_db = temp.path().join("drifted.db");
    std::fs::create_dir_all(&project).expect("create registered project");
    register_project(RegisterOptions {
        brain_home: brain_home.clone(),
        project_path: project,
        claude_projects_root: None,
        explicit_claude_sources: Vec::new(),
        pipe_name: None,
    })
    .expect("register project");
    rusqlite::Connection::open(&reviewed_db)
        .expect("open reviewed Hermes database")
        .execute_batch(include_str!("../../../fixtures/hermes/schema.sql"))
        .expect("apply reviewed Hermes schema");
    rusqlite::Connection::open(&drifted_db)
        .expect("open drifted Hermes database")
        .execute_batch(include_str!("../../../fixtures/hermes/schema-drift.sql"))
        .expect("apply drifted Hermes schema");

    let active =
        read_hermes_status(&brain_home, &reviewed_db, None).expect("read active Hermes status");
    assert_eq!(active.activation, "active");
    assert_eq!(active.expected_fingerprint, active.observed_fingerprint);
    assert!(active.reason.is_none());

    let guarded =
        read_hermes_status(&brain_home, &drifted_db, None).expect("read guarded Hermes status");
    assert_eq!(guarded.activation, "fixture_only");
    assert_ne!(guarded.expected_fingerprint, guarded.observed_fingerprint);
    assert!(guarded.reason.is_some());
}
