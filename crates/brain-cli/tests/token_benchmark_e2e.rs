use std::collections::BTreeMap;
use std::path::Path;

use brain_cli::{
    BenchmarkArtifacts, BenchmarkCondition, BenchmarkHarness, BenchmarkPreflightReport,
    BenchmarkStatus, ConditionDiff, GradeOutcome, GradeRecord, NativeUsage, PlannedSample,
    ProductionConfigHashes, RunManifest, SampleRecord, SampleStatus, ValidityCheck,
    build_report_from_artifacts,
};
use brain_domain::ProjectId;

#[test]
fn synthetic_run_reproduces_quality_blocked_report_without_dropping_failed_answer_tokens() {
    let temp = tempfile::tempdir().expect("temp");
    let project = ProjectId(uuid::Uuid::now_v7());
    let run = uuid::Uuid::now_v7();
    let artifacts = BenchmarkArtifacts::new(temp.path(), project, run).expect("artifacts");
    let matrix = vec![
        planned(
            "claude-off",
            "claude-pair",
            "memory",
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::BrainOff,
            0,
        ),
        planned(
            "claude-on",
            "claude-pair",
            "memory",
            BenchmarkHarness::ClaudeCode,
            BenchmarkCondition::BrainOn,
            1,
        ),
        planned(
            "codex-off",
            "codex-pair",
            "navigate",
            BenchmarkHarness::Codex,
            BenchmarkCondition::BrainOff,
            0,
        ),
        planned(
            "codex-on",
            "codex-pair",
            "navigate",
            BenchmarkHarness::Codex,
            BenchmarkCondition::BrainOn,
            1,
        ),
    ];
    artifacts
        .create_run(&RunManifest {
            schema_version: 1,
            run_id: run,
            project_id: project,
            suite_id: "synthetic-v1".to_owned(),
            suite_sha256: "a".repeat(64),
            repository_commit: "b".repeat(40),
            brain_commit: "c".repeat(40),
            frozen_snapshot_sha256: "d".repeat(64),
            seed: 42,
            repeats: 1,
            bootstrap_resamples: 1_000,
            claimable: false,
            created_at: "2026-08-11T00:00:00Z".to_owned(),
            matrix: matrix.clone(),
        })
        .expect("manifest");
    let suite = serde_json::json!({
        "schema_version": 1,
        "suite_id": "synthetic-v1",
        "pilot_task_ids": [],
        "external_references": [
            {
                "system": "agentmemory",
                "source_url": "https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/benchmark/COMPARISON.md",
                "source_revision": "2973e4ec4c40d323a08daa34220118010e73a2c3",
                "source_sha256": "fa989e41dc0b9a581e365a7993cf9d666169e6bfc2690b754ba5f5d6eb5e10dd",
                "claims": [
                    {
                        "benchmark": "Annual token model",
                        "metric": "tokens/year",
                        "value": "~170K",
                        "evidence_class": "vendor-modeled estimate",
                        "comparability": "Not native Claude/Codex usage; context-only reference"
                    }
                ]
            }
        ],
        "tasks": [
            synthetic_task("memory", "historical_recall"),
            synthetic_task("navigate", "codebase_navigation")
        ]
    });
    artifacts
        .write_immutable_file(
            Path::new("suite.json"),
            &serde_json::to_vec_pretty(&suite).unwrap(),
        )
        .expect("suite");
    for (path, model) in [
        ("configs/claude-c0-native-default.json", "claude-fixture"),
        ("configs/codex-c0-native-default.json", "codex-fixture"),
    ] {
        artifacts
            .write_immutable_file(
                Path::new(path),
                &serde_json::to_vec_pretty(&serde_json::json!({ "model": model })).unwrap(),
            )
            .expect("config");
    }
    let condition = ConditionDiff {
        passed: true,
        allowed_paths: Vec::new(),
        changed_paths: Vec::new(),
        unexpected_paths: Vec::new(),
    };
    artifacts
        .write_named_json(
            "preflight",
            &BenchmarkPreflightReport {
                schema_version: 1,
                run_id: run,
                valid: true,
                paid_sessions_launched: 0,
                suite_sha256: "a".repeat(64),
                repository_commit: "b".repeat(40),
                brain_commit: "c".repeat(40),
                snapshot_sha256: "d".repeat(64),
                execution_templates_sha256: "e".repeat(64),
                pipe_name: r"\\.\pipe\synthetic".to_owned(),
                executable_pins: BTreeMap::new(),
                claude_condition_diff: condition.clone(),
                codex_condition_diff: condition,
                condition_profiles: None,
                production_config_hashes: ProductionConfigHashes {
                    claude_settings: None,
                    claude_mcp: None,
                    codex_hooks: None,
                    codex_config: None,
                },
                checkout: temp.path().join("checkout"),
                frozen_brain_home: temp.path().join("frozen"),
                validity_checks: vec![ValidityCheck {
                    name: "synthetic_preflight".to_owned(),
                    passed: true,
                    detail: "zero-cost fixture".to_owned(),
                }],
            },
        )
        .expect("preflight");

    for (sample, tokens, answer) in [
        (&matrix[0], 100, "correct control"),
        (&matrix[1], 80, "wrong but cheaper treatment"),
        (&matrix[2], 200, "correct control"),
        (&matrix[3], 150, "correct treatment"),
    ] {
        artifacts
            .append_sample(&sample_record(sample, tokens, answer))
            .expect("sample");
    }
    std::fs::write(
        artifacts.run_dir().join("grading-key.csv"),
        "opaque_id,sample_id\no1,claude-off\no2,claude-on\no3,codex-off\no4,codex-on\n",
    )
    .expect("grading key");
    for (opaque, outcome) in [
        ("o1", GradeOutcome::Pass),
        ("o2", GradeOutcome::Fail),
        ("o3", GradeOutcome::Pass),
        ("o4", GradeOutcome::Pass),
    ] {
        artifacts
            .append_grade(&GradeRecord {
                schema_version: 1,
                opaque_id: opaque.to_owned(),
                outcome,
                critical_regression: false,
                reason: "synthetic grade".to_owned(),
                grader: "fixture".to_owned(),
                grader_version: "1".to_owned(),
                graded_at: "2026-08-11T01:00:00Z".to_owned(),
            })
            .expect("grade");
    }

    let report = build_report_from_artifacts(temp.path(), project, run).expect("report");
    assert_eq!(report.status, BenchmarkStatus::QualityBlocked);
    let overall = report.overall.expect("overall tokens");
    assert_eq!(overall.control_tokens, 300);
    assert_eq!(overall.treatment_tokens, 230);
    assert_eq!(overall.saved_tokens, 70);
    assert_eq!(
        report.statement,
        "Lower token usage was observed, but the quality gate failed. No token-savings claim is valid for this run."
    );
    let summary = artifacts.summary().expect("summary");
    assert_eq!(summary.pair_audit.len(), 2);
    assert_eq!(summary.pair_audit[0].treatment_grade, GradeOutcome::Fail);
    let dashboard_json = serde_json::to_string(&summary).expect("dashboard JSON");
    assert!(!dashboard_json.contains("wrong but cheaper treatment"));
    assert!(!dashboard_json.contains("correct control"));

    let benchmark = std::fs::read_to_string(artifacts.run_dir().join("BENCHMARK.md"))
        .expect("benchmark markdown");
    assert!(benchmark.contains("External published context (not a C0-C4 result)"));
    assert!(benchmark.contains("| agentmemory | Annual token model | tokens/year | ~170K |"));
    assert!(benchmark.contains("never populate Goal 1, Goal 2, or Goal 3"));
}

fn planned(
    sample_id: &str,
    pair_id: &str,
    task_id: &str,
    harness: BenchmarkHarness,
    condition: BenchmarkCondition,
    order: u8,
) -> PlannedSample {
    PlannedSample {
        sample_id: sample_id.to_owned(),
        pair_id: pair_id.to_owned(),
        task_id: task_id.to_owned(),
        harness,
        repeat: 1,
        condition,
        order,
    }
}

fn sample_record(sample: &PlannedSample, total_tokens: u64, answer: &str) -> SampleRecord {
    SampleRecord {
        schema_version: 1,
        sample: sample.clone(),
        status: SampleStatus::Completed,
        attempt: 1,
        native_usage: Some(NativeUsage {
            harness: sample.harness,
            input_tokens: total_tokens.saturating_sub(10),
            cache_creation_input_tokens: (sample.harness == BenchmarkHarness::ClaudeCode)
                .then_some(0),
            cache_read_input_tokens: (sample.harness == BenchmarkHarness::ClaudeCode).then_some(0),
            cached_input_tokens: (sample.harness == BenchmarkHarness::Codex).then_some(0),
            cache_write_input_tokens: None,
            output_tokens: 10,
            reasoning_output_tokens: None,
            total_tokens,
            native_records: 1,
        }),
        elapsed_ms: Some(100),
        native_trace: None,
        answer: answer.to_owned(),
        automated_test_passed: None,
        stdout_sha256: "1".repeat(64),
        stderr_sha256: "2".repeat(64),
        error: None,
        completed_at: "2026-08-11T00:30:00Z".to_owned(),
    }
}

fn synthetic_task(id: &str, stratum: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "stratum": stratum,
        "prompt": "fixture prompt",
        "fixture_commit": "b".repeat(40),
        "allowed_files": [],
        "rubric": "fixture.md",
        "reference_facts": [],
        "automated_check": null,
        "critical_regression": "none",
        "combined_eligible": true,
        "max_turns": 1,
        "max_tool_calls": 1,
        "timeout_seconds": 30
    })
}
