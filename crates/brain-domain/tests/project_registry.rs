use brain_domain::ProjectRegistry;

#[test]
fn registration_survives_reload_with_the_same_ids() {
    let temp = tempfile::tempdir().expect("create registry fixture");
    let brain_home = temp.path().join("brain");
    let project = temp.path().join("projects").join("api");
    std::fs::create_dir_all(&project).expect("create project root");

    let first = ProjectRegistry::open(&brain_home)
        .expect("open registry")
        .register(&project)
        .expect("register project");
    let second = ProjectRegistry::open(&brain_home)
        .expect("reload registry")
        .register(&project)
        .expect("register project again");

    assert_eq!(first.project_id, second.project_id);
    assert_eq!(first.worktree_id, second.worktree_id);
    assert!(brain_home.join("projects.json").is_file());
    assert!(!brain_home.join("projects.json.tmp").exists());
}

#[test]
fn an_explicit_alias_keeps_project_identity_after_a_move() {
    let temp = tempfile::tempdir().expect("create registry fixture");
    let brain_home = temp.path().join("brain");
    let original = temp.path().join("original");
    let moved = temp.path().join("moved");
    std::fs::create_dir_all(&original).expect("create original project root");

    let registered = ProjectRegistry::open(&brain_home)
        .expect("open registry")
        .register(&original)
        .expect("register original project");
    std::fs::rename(&original, &moved).expect("move project root");

    let aliased = ProjectRegistry::open(&brain_home)
        .expect("reload registry")
        .add_alias(registered.project_id, &moved)
        .expect("add moved path alias");
    let resolved = ProjectRegistry::open(&brain_home)
        .expect("reload aliased registry")
        .register(&moved)
        .expect("resolve moved project");

    assert_eq!(registered.project_id, aliased.project_id);
    assert_eq!(registered.project_id, resolved.project_id);
}

#[test]
fn an_alias_already_owned_by_another_project_is_rejected() {
    let temp = tempfile::tempdir().expect("create registry fixture");
    let brain_home = temp.path().join("brain");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir_all(&first).expect("create first project root");
    std::fs::create_dir_all(&second).expect("create second project root");

    let first_identity = ProjectRegistry::open(&brain_home)
        .expect("open registry")
        .register(&first)
        .expect("register first project");
    ProjectRegistry::open(&brain_home)
        .expect("reload registry")
        .register(&second)
        .expect("register second project");

    let error = ProjectRegistry::open(&brain_home)
        .expect("reload registry")
        .add_alias(first_identity.project_id, &second)
        .expect_err("ambiguous alias must fail");

    assert!(error.to_string().contains("already belongs to project"));
}

#[test]
fn a_newer_registry_schema_is_rejected_instead_of_guessed() {
    let temp = tempfile::tempdir().expect("create registry fixture");
    let brain_home = temp.path().join("brain");
    std::fs::create_dir_all(&brain_home).expect("create brain home");
    std::fs::write(
        brain_home.join("projects.json"),
        r#"{"schema_version":999,"projects":[]}"#,
    )
    .expect("write newer registry fixture");

    let error = match ProjectRegistry::open(&brain_home) {
        Ok(_) => panic!("newer schema must fail"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("unsupported project registry schema")
    );
}
