use brain_cli::{
    BenchmarkArtifacts, BenchmarkCondition, BenchmarkHarness, NativeUsage, PlannedSample,
    RunManifest, SampleRecord, SampleStatus,
};
use brain_domain::ProjectId;

#[test]
fn artifacts_are_project_scoped_append_only_and_resumable() {
    let temp = tempfile::tempdir().unwrap();
    let project = ProjectId(uuid::Uuid::now_v7());
    let other = ProjectId(uuid::Uuid::now_v7());
    let run_id = uuid::Uuid::now_v7();
    let store = BenchmarkArtifacts::new(temp.path(), project, run_id).unwrap();
    let manifest = manifest(project, run_id);
    store.create_run(&manifest).unwrap();
    store.create_run(&manifest).unwrap();
    let sample = sample();
    store.append_sample(&sample).unwrap();
    store.append_sample(&sample).unwrap();
    assert_eq!(store.samples().unwrap(), vec![sample.clone()]);

    let mut conflict = sample;
    conflict.answer = "different".to_owned();
    assert!(
        store
            .append_sample(&conflict)
            .unwrap_err()
            .to_string()
            .contains("conflicting")
    );
    let mut retry = conflict;
    retry.attempt = 2;
    store.append_sample(&retry).unwrap();
    assert_eq!(
        store.samples().unwrap().len(),
        2,
        "a retry is append-only history, not a conflicting rewrite"
    );
    assert!(
        BenchmarkArtifacts::new(temp.path(), other, run_id)
            .unwrap()
            .samples()
            .unwrap()
            .is_empty()
    );

    store.retire("superseded by a corrected run").unwrap();
    assert!(store.is_retired());
    assert!(store.manifest_path().is_file());
}

#[test]
fn raw_output_is_content_addressed() {
    let temp = tempfile::tempdir().unwrap();
    let store = BenchmarkArtifacts::new(
        temp.path(),
        ProjectId(uuid::Uuid::now_v7()),
        uuid::Uuid::now_v7(),
    )
    .unwrap();
    let first = store
        .write_raw("sample-1", "stdout", b"same bytes")
        .unwrap();
    let second = store
        .write_raw("sample-1", "stdout", b"same bytes")
        .unwrap();
    assert_eq!(first, second);
    assert!(store.run_dir().join(first.relative_path).is_file());
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
        native_usage: Some(NativeUsage {
            harness: BenchmarkHarness::ClaudeCode,
            input_tokens: 10,
            cache_creation_input_tokens: Some(1),
            cache_read_input_tokens: Some(2),
            cached_input_tokens: None,
            cache_write_input_tokens: None,
            output_tokens: 3,
            reasoning_output_tokens: None,
            total_tokens: 16,
            native_records: 1,
        }),
        answer: "answer".to_owned(),
        automated_test_passed: None,
        stdout_sha256: "e".repeat(64),
        stderr_sha256: "f".repeat(64),
        error: None,
        completed_at: "2026-08-11T00:01:00Z".to_owned(),
    }
}
