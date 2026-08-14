use std::collections::BTreeMap;

use brain_cli::{
    BenchmarkArtifacts, BenchmarkCondition, BenchmarkHarness, PlannedSample, RunManifest,
    SampleRecord, SampleStatus, export_grading_bundle, import_grades,
};
use brain_domain::ProjectId;

#[test]
fn public_grading_sheet_hides_harness_and_condition() {
    let temp = tempfile::tempdir().unwrap();
    let project = ProjectId(uuid::Uuid::now_v7());
    let run = uuid::Uuid::now_v7();
    let artifacts = BenchmarkArtifacts::new(temp.path(), project, run).unwrap();
    artifacts.create_run(&manifest(project, run)).unwrap();
    artifacts.append_sample(&sample()).unwrap();
    let mut task_text = BTreeMap::new();
    task_text.insert(
        "task-1".to_owned(),
        (
            "Explain the code.".to_owned(),
            "Must identify the boundary.".to_owned(),
        ),
    );
    let exported = export_grading_bundle(&artifacts, &task_text, 42).unwrap();
    let sheet = std::fs::read_to_string(exported.sheet).unwrap();
    let key = std::fs::read_to_string(exported.key).unwrap();
    assert!(!sheet.contains("claude_code"));
    assert!(!sheet.contains("brain_off"));
    assert!(!sheet.contains("sample-1"));
    assert!(key.contains("sample-1"));
}

#[test]
fn imported_grades_are_append_only_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let project = ProjectId(uuid::Uuid::now_v7());
    let run = uuid::Uuid::now_v7();
    let artifacts = BenchmarkArtifacts::new(temp.path(), project, run).unwrap();
    artifacts.create_run(&manifest(project, run)).unwrap();
    let grades = temp.path().join("grades.csv");
    std::fs::write(
        &grades,
        "opaque_id,outcome,critical_regression,reason\nopaque-1,pass,false,correct\n",
    )
    .unwrap();
    import_grades(&artifacts, &grades, "JG", "1", "2026-08-11T00:00:00Z").unwrap();
    import_grades(&artifacts, &grades, "JG", "1", "2026-08-11T00:00:00Z").unwrap();
    assert_eq!(artifacts.grades().unwrap().len(), 1);
    std::fs::write(
        &grades,
        "opaque_id,outcome,critical_regression,reason\nopaque-1,fail,false,wrong\n",
    )
    .unwrap();
    assert!(import_grades(&artifacts, &grades, "JG", "1", "2026-08-11T00:00:00Z").is_err());
}

fn manifest(project_id: ProjectId, run_id: uuid::Uuid) -> RunManifest {
    RunManifest {
        schema_version: 1,
        run_id,
        project_id,
        suite_id: "test".to_owned(),
        suite_sha256: "a".repeat(64),
        repository_commit: "b".repeat(40),
        brain_commit: "c".repeat(40),
        frozen_snapshot_sha256: "d".repeat(64),
        seed: 42,
        repeats: 1,
        bootstrap_resamples: 10_000,
        claimable: false,
        created_at: "2026-08-11T00:00:00Z".to_owned(),
        matrix: vec![],
    }
}

fn sample() -> SampleRecord {
    SampleRecord {
        schema_version: 1,
        sample: PlannedSample {
            sample_id: "sample-1".to_owned(),
            pair_id: "pair-1".to_owned(),
            task_id: "task-1".to_owned(),
            harness: BenchmarkHarness::ClaudeCode,
            repeat: 1,
            condition: BenchmarkCondition::BrainOff,
            order: 0,
        },
        status: SampleStatus::Completed,
        attempt: 1,
        native_usage: None,
        elapsed_ms: Some(100),
        native_trace: None,
        answer: "The answer.".to_owned(),
        automated_test_passed: None,
        stdout_sha256: "a".repeat(64),
        stderr_sha256: "b".repeat(64),
        error: None,
        completed_at: "2026-08-11T00:00:00Z".to_owned(),
    }
}
