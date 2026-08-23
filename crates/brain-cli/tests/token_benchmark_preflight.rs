use std::fs;

use brain_cli::{
    compare_condition_configs, freeze_project_snapshot, hash_optional_file,
    validate_condition_profiles,
};
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

#[test]
fn v2_profiles_have_exact_cumulative_capabilities_and_native_defaults() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/second-brain/v2/configs");
    let report = validate_condition_profiles(&root).unwrap();
    assert!(report.valid, "{:?}", report.errors);
    assert_eq!(report.profile_sha256.len(), 10);
    assert_eq!(report.diffs.len(), 8);
}

#[test]
fn a_reachable_brain_in_c0_or_model_drift_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/second-brain/v2/configs");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), temp.path().join(entry.file_name())).unwrap();
    }
    let c0 = temp.path().join("codex-c0-native-default.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&c0).unwrap()).unwrap();
    value["mcp"] = serde_json::json!({"agent_brain":{"transport":"brain-mcp.exe"}});
    fs::write(&c0, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let report = validate_condition_profiles(temp.path()).unwrap();
    assert!(!report.valid);
    assert!(report.errors.iter().any(|error| error.contains("c0")));

    value["mcp"] = serde_json::json!({});
    value["model"] = serde_json::json!("different-model");
    fs::write(&c0, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let report = validate_condition_profiles(temp.path()).unwrap();
    assert!(!report.valid);
    assert!(report.errors.iter().any(|error| error.contains("model")));
}
