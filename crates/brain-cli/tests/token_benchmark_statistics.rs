use brain_cli::{
    BenchmarkHarness, BenchmarkPair, GradeOutcome, clustered_estimate, evaluate_benchmark,
};

fn pair(task: &str, harness: BenchmarkHarness, off: u64, on: u64) -> BenchmarkPair {
    BenchmarkPair {
        task_id: task.to_owned(),
        harness,
        repeat: 1,
        control_tokens: off,
        treatment_tokens: on,
        control_grade: GradeOutcome::Pass,
        treatment_grade: GradeOutcome::Pass,
        treatment_critical_regression: false,
    }
}

#[test]
fn estimate_is_ratio_of_sums_not_average_pair_percentages() {
    let pairs = vec![
        pair("small", BenchmarkHarness::ClaudeCode, 100, 50),
        pair("large", BenchmarkHarness::ClaudeCode, 900, 810),
    ];
    let estimate = clustered_estimate(&pairs, 10_000, 42).expect("estimate");
    assert_eq!(estimate.control_tokens, 1_000);
    assert_eq!(estimate.treatment_tokens, 860);
    assert_eq!(estimate.saved_tokens, 140);
    assert!((estimate.savings_fraction - 0.14).abs() < 1e-12);
    assert_ne!(estimate.savings_fraction, 0.30);
}

#[test]
fn clustered_bootstrap_is_deterministic() {
    let pairs = vec![
        pair("a", BenchmarkHarness::Codex, 1_000, 700),
        pair("b", BenchmarkHarness::Codex, 800, 720),
        pair("c", BenchmarkHarness::Codex, 1_200, 960),
    ];
    assert_eq!(
        clustered_estimate(&pairs, 10_000, 8675309).unwrap(),
        clustered_estimate(&pairs, 10_000, 8675309).unwrap()
    );
}

#[test]
fn quality_failure_does_not_remove_tokens_and_blocks_claim() {
    let mut pairs = vec![
        pair("a", BenchmarkHarness::ClaudeCode, 1_000, 500),
        pair("a", BenchmarkHarness::Codex, 1_000, 500),
    ];
    pairs[0].treatment_grade = GradeOutcome::Fail;
    let report = evaluate_benchmark(&pairs, &[], 10_000, 42, 1).expect("report");
    assert_eq!(report.overall.as_ref().unwrap().treatment_tokens, 1_000);
    assert_eq!(report.status.as_str(), "quality_blocked");
    assert!(report.statement.contains("quality gate failed"));
}

#[test]
fn an_interval_touching_zero_is_not_proven() {
    let pairs = vec![
        pair("a", BenchmarkHarness::ClaudeCode, 1_000, 1_000),
        pair("a", BenchmarkHarness::Codex, 1_000, 1_000),
    ];
    let report = evaluate_benchmark(&pairs, &[], 10_000, 42, 1).unwrap();
    assert_eq!(report.status.as_str(), "inconclusive");
}

#[test]
fn any_failed_validity_check_makes_the_run_invalid() {
    let pairs = vec![
        pair("a", BenchmarkHarness::ClaudeCode, 1_000, 500),
        pair("a", BenchmarkHarness::Codex, 1_000, 500),
    ];
    let report = evaluate_benchmark(
        &pairs,
        &[brain_cli::ValidityCheck {
            name: "condition_diff".to_owned(),
            passed: false,
            detail: "CodeGraph differed".to_owned(),
        }],
        10_000,
        42,
        1,
    )
    .unwrap();
    assert_eq!(report.status.as_str(), "invalid");
    assert!(report.statement.contains("condition_diff"));
}
