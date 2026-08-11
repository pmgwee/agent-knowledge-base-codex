use brain_cli::{
    BenchmarkCondition, BenchmarkHarness, BenchmarkTask, BenchmarkTaskStratum, SuiteManifest,
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
                automated_check: None,
                critical_regression: "No materially false claim.".to_owned(),
                combined_eligible: true,
                max_turns: 8,
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
