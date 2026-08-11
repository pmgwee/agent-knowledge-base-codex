use std::collections::BTreeMap;
use std::path::PathBuf;

use brain_cli::{
    BenchmarkCondition, BenchmarkHarness, BenchmarkSummary, BenchmarkTask, BenchmarkTaskStratum,
    ExecutionTemplate, ExecutionTemplates, HarnessExecutionTemplates, SuiteManifest,
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
            });
        }
    }
    let suite = SuiteManifest {
        schema_version: 1,
        suite_id: "token-savings-v1".to_owned(),
        pilot_task_ids: tasks.iter().take(4).map(|task| task.id.clone()).collect(),
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
    assert_eq!(
        serde_json::to_string(&BenchmarkCondition::BrainOff).unwrap(),
        "\"brain_off\""
    );
    assert_eq!(
        serde_json::to_string(&BenchmarkCondition::BrainOn).unwrap(),
        "\"brain_on\""
    );
}

#[test]
fn execution_profiles_select_the_exact_harness_and_condition() {
    let common = ExecutionTemplate {
        program: PathBuf::from("launcher.exe"),
        args: vec!["{{prompt}}".to_owned(), "{{condition_config}}".to_owned()],
        environment: BTreeMap::new(),
        timeout_seconds: 600,
    };
    let mut treatment = common.clone();
    treatment
        .environment
        .insert("BRAIN_HOME".to_owned(), "{{sample_brain_home}}".to_owned());
    treatment
        .environment
        .insert("BRAIN_PIPE_NAME".to_owned(), "{{pipe_name}}".to_owned());
    let templates = ExecutionTemplates {
        schema_version: 1,
        max_attempts: 2,
        brain_service_program: PathBuf::from("brain-service.exe"),
        claude_code: HarnessExecutionTemplates {
            brain_off: common.clone(),
            brain_on: treatment.clone(),
        },
        codex: HarnessExecutionTemplates {
            brain_off: common,
            brain_on: treatment.clone(),
        },
    };
    assert_eq!(
        templates.template(BenchmarkHarness::Codex, BenchmarkCondition::BrainOn),
        &treatment
    );
    assert_eq!(
        serde_json::from_str::<ExecutionTemplates>(&serde_json::to_string(&templates).unwrap())
            .unwrap(),
        templates
    );
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
