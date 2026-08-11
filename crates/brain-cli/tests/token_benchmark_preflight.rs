use std::fs;

use brain_cli::{compare_condition_configs, freeze_project_snapshot, hash_optional_file};
use brain_domain::ProjectId;

#[test]
fn condition_diff_allows_only_declared_brain_paths() {
    let control: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/token-benchmark/condition-control.json"
    ))
    .unwrap();
    let treatment: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/token-benchmark/condition-treatment.json"
    ))
    .unwrap();
    let report = compare_condition_configs(
        &control,
        &treatment,
        &[
            "/hooks/agent_brain",
            "/mcp/agent_brain",
            "/environment/BRAIN_PIPE_NAME",
        ],
    );
    assert!(report.passed, "{:?}", report.unexpected_paths);

    let mut drifted = treatment;
    drifted["model"] = serde_json::json!("different-model");
    let report = compare_condition_configs(&control, &drifted, &["/hooks/agent_brain"]);
    assert!(!report.passed);
    assert!(report.unexpected_paths.contains(&"/model".to_owned()));
}

#[test]
fn snapshot_copy_is_immutable_and_hashed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("ledger.sqlite"), b"ledger").unwrap();
    fs::write(source.join("nested/live.json"), b"live").unwrap();
    let destination = temp.path().join("run/frozen");
    let snapshot =
        freeze_project_snapshot(&source, &destination, ProjectId(uuid::Uuid::now_v7())).unwrap();
    assert_eq!(snapshot.files, 2);
    assert_eq!(snapshot.sha256.len(), 64);
    assert_eq!(
        fs::read(destination.join("ledger.sqlite")).unwrap(),
        b"ledger"
    );
}

#[test]
fn absent_and_present_config_hashes_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    assert_eq!(hash_optional_file(&path).unwrap(), None);
    fs::write(&path, b"{}").unwrap();
    assert_eq!(hash_optional_file(&path).unwrap().unwrap().len(), 64);
}
