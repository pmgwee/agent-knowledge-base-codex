use std::collections::BTreeMap;
use std::path::PathBuf;

use brain_cli::{
    BenchmarkCondition, BenchmarkHarness, BenchmarkSummary, BenchmarkTask, BenchmarkTaskStratum,
    ExecutionTemplate, ExecutionTemplates, ExternalBenchmarkClaim, ExternalBenchmarkReference,
    HarnessExecutionTemplates, PRIMARY_CONTRASTS, SuiteManifest,
};

#[test]
fn suite_validation_requires_unique_balanced_tasks() {
    let mut tasks = Vec::new();
    for (prefix, stratum) in [
        ("memory", BenchmarkTaskStratum::HistoricalRecall),
        ("navigate", BenchmarkTaskStratum::CodebaseNavigation),
        ("diagnose", BenchmarkTaskStratum::DiagnosisPlanning),
        ("change", BenchmarkTaskStratum::BoundedChange),
    ] {
        for index in 1..=2 {
            tasks.push(BenchmarkTask {
                id: format!("{prefix}-{index:02}"),
                stratum,
                prompt: "Answer the bounded task.".to_owned(),
                fixture_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                allowed_files: vec!["crates/**".to_owned()],
                rubric: "rubrics/example.md".to_owned(),
                reference_facts: vec!["A cited fact.".to_owned()],
                automated_check: None,
                critical_regression: "No materially false claim.".to_owned(),
                combined_eligible: true,
                max_turns: 8,
                max_tool_calls: 20,
                timeout_seconds: 600,
                trace_markers: None,
            });
        }
    }
    let suite = SuiteManifest {
        schema_version: 2,
        suite_id: "second-brain-five-condition-v2".to_owned(),
        pilot_task_ids: tasks.iter().take(4).map(|task| task.id.clone()).collect(),
        external_references: Vec::new(),
        tasks,
    };

    suite.validate().expect("balanced suite");
    let encoded = serde_json::to_string(&suite).expect("serialize");
    let decoded: SuiteManifest = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded, suite);

    let mut duplicate = suite.clone();
    duplicate.tasks[1].id = duplicate.tasks[0].id.clone();
    assert!(
        duplicate
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );

    let mut malformed_reference = suite;
    malformed_reference
        .external_references
        .push(ExternalBenchmarkReference {
            system: "competitor".to_owned(),
            source_url: "https://example.test/scorecard".to_owned(),
            source_revision: "pinned".to_owned(),
            source_sha256: "too-short".to_owned(),
            claims: vec![ExternalBenchmarkClaim {
                benchmark: "fixture".to_owned(),
                metric: "R@5".to_owned(),
                value: "95.0%".to_owned(),
                evidence_class: "external".to_owned(),
                comparability: "independent harness".to_owned(),
            }],
        });
    assert!(
        malformed_reference
            .validate()
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
}

#[test]
fn harness_and_condition_names_are_stable() {
    assert_eq!(
        serde_json::to_string(&BenchmarkHarness::ClaudeCode).unwrap(),
        "\"claude_code\""
    );
    assert_eq!(
        serde_json::to_string(&BenchmarkHarness::Codex).unwrap(),
        "\"codex\""
    );
    assert_eq!(BenchmarkCondition::ALL.len(), 5);
    assert_eq!(
        BenchmarkCondition::ALL
            .into_iter()
            .map(|condition| serde_json::to_string(&condition).unwrap())
            .collect::<Vec<_>>(),
        ["\"c0\"", "\"c1\"", "\"c2\"", "\"c3\"", "\"c4\""]
    );
    assert_eq!(
        serde_json::from_str::<BenchmarkCondition>("\"brain_off\"").unwrap(),
        BenchmarkCondition::C0
    );
    assert_eq!(
        serde_json::from_str::<BenchmarkCondition>("\"brain_on\"").unwrap(),
        BenchmarkCondition::C4
    );
    assert_eq!(PRIMARY_CONTRASTS.len(), 6);
    assert!(PRIMARY_CONTRASTS.iter().any(|contrast| {
        contrast.id == "c4_vs_c0"
            && contrast.baseline == BenchmarkCondition::C0
            && contrast.treatment == BenchmarkCondition::C4
    }));
}

#[test]
fn execution_profiles_select_the_exact_harness_and_condition() {
    let common = ExecutionTemplate {
        program: PathBuf::from("launcher.exe"),
        args: vec!["{{prompt}}".to_owned(), "{{condition_config}}".to_owned()],
        environment: BTreeMap::new(),
        timeout_seconds: 600,
    };
    let condition_templates = BenchmarkCondition::ALL
        .into_iter()
        .map(|condition| {
            let mut template = common.clone();
            template.environment.insert(
                "BENCHMARK_CONDITION".to_owned(),
                condition.as_str().to_owned(),
            );
            (condition, template)
        })
        .collect::<BTreeMap<_, _>>();
    let templates = ExecutionTemplates {
        schema_version: 2,
        max_attempts: 2,
        brain_service_program: PathBuf::from("brain-service.exe"),
        launcher_environment: BTreeMap::new(),
        claude_code: HarnessExecutionTemplates {
            conditions: condition_templates.clone(),
        },
        codex: HarnessExecutionTemplates {
            conditions: condition_templates.clone(),
        },
    };
    templates.validate().expect("complete five-condition maps");
    assert_eq!(
        templates
            .template(BenchmarkHarness::Codex, BenchmarkCondition::C4)
            .environment["BENCHMARK_CONDITION"],
        "c4"
    );
    assert_eq!(
        serde_json::from_str::<ExecutionTemplates>(&serde_json::to_string(&templates).unwrap())
            .unwrap(),
        templates
    );

    let missing = ExecutionTemplates {
        codex: HarnessExecutionTemplates {
            conditions: BTreeMap::from([(BenchmarkCondition::C0, common.clone())]),
        },
        ..templates.clone()
    };
    assert!(missing.validate().unwrap_err().to_string().contains("c1"));

    let legacy: ExecutionTemplates = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "max_attempts": 2,
        "brain_service_program": "brain-service.exe",
        "claude_code": {"brain_off": common, "brain_on": condition_templates[&BenchmarkCondition::C4]},
        "codex": {"brain_off": condition_templates[&BenchmarkCondition::C0], "brain_on": condition_templates[&BenchmarkCondition::C4]}
    }))
    .expect("v1 execution profile remains readable");
    assert_eq!(legacy.claude_code.conditions.len(), 2);
    legacy.validate().expect("legacy c0/c4 maps remain valid");
}

#[test]
fn older_summary_without_audit_metadata_remains_readable() {
    let summary: BenchmarkSummary = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "run_id": uuid::Uuid::now_v7(),
        "status": "invalid",
        "statement": "No claim.",
        "completed_at": "2026-08-11T00:00:00Z",
        "overall": null,
        "harnesses": [],
        "overall_quality": null,
        "validity_checks": []
    }))
    .expect("old summary");
    assert!(summary.metadata.suite_id.is_empty());
    assert!(summary.pair_audit.is_empty());
}
