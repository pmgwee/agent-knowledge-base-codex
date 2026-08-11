use std::process::Command;

use brain_cli::{BenchmarkArtifacts, RunManifest};
use brain_domain::ProjectId;

#[test]
fn benchmark_help_exposes_scale_and_token_workflows() {
    let output = Command::new(env!("CARGO_BIN_EXE_brain"))
        .args(["benchmark", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for command in [
        "scale",
        "preflight",
        "run",
        "grade",
        "report",
        "show",
        "retire",
        "production",
    ] {
        assert!(text.contains(command), "missing {command}: {text}");
    }
}

#[test]
fn run_without_execute_launches_zero_paid_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let project = ProjectId(uuid::Uuid::now_v7());
    let run = uuid::Uuid::now_v7();
    let artifacts = BenchmarkArtifacts::new(temp.path(), project, run).unwrap();
    artifacts
        .create_run(&RunManifest {
            schema_version: 1,
            run_id: run,
            project_id: project,
            suite_id: "fixture".to_owned(),
            suite_sha256: "a".repeat(64),
            repository_commit: "b".repeat(40),
            brain_commit: "c".repeat(40),
            frozen_snapshot_sha256: "d".repeat(64),
            seed: 42,
            repeats: 1,
            bootstrap_resamples: 10_000,
            claimable: false,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
            matrix: Vec::new(),
        })
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_brain"))
        .arg("--brain-home")
        .arg(temp.path())
        .args([
            "benchmark",
            "run",
            "--project",
            &project.0.to_string(),
            "--run",
            &run.to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["execute"], false);
    assert_eq!(value["paid_sessions_launched"], 0);
}
