use brain_cli::{
    BenchmarkCondition, BenchmarkHarness, BenchmarkObservation, BenchmarkTaskStratum, GoalStatus,
    GradeOutcome, ThreeGoalEvaluationOptions, evaluate_three_goal_benchmark,
};

#[test]
fn an_empty_run_renders_not_measured_for_every_goal_and_contrast() {
    let report = evaluate_three_goal_benchmark(
        &[],
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: 10_000,
            seed: 42,
        },
    )
    .unwrap();
    assert!(report.statement.starts_with("Not measured"));
    assert_eq!(report.contrasts.len(), 6);
    assert!(report.contrasts.iter().all(|contrast| {
        contrast.tokens.status == GoalStatus::NotRun
            && contrast.speed.status == GoalStatus::NotRun
            && contrast.quality.status == GoalStatus::NotRun
    }));
}

#[test]
fn a_treatment_only_critical_regression_is_visible_in_all_performance_claims() {
    let mut observations = Vec::new();
    for harness in BenchmarkHarness::ALL {
        for condition in BenchmarkCondition::ALL {
            observations.push(BenchmarkObservation {
                task_id: "critical".to_owned(),
                harness,
                repeat: 1,
                stratum: BenchmarkTaskStratum::HistoricalRecall,
                condition,
                native_tokens: Some(if condition == BenchmarkCondition::C4 {
                    50
                } else {
                    100
                }),
                elapsed_ms: Some(if condition == BenchmarkCondition::C4 {
                    50
                } else {
                    100
                }),
                grade: if condition == BenchmarkCondition::C4 {
                    GradeOutcome::Fail
                } else {
                    GradeOutcome::Pass
                },
                critical_regression: condition == BenchmarkCondition::C4,
                timed_out: false,
                harness_failed: false,
                brain_mcp_calls: 0,
            });
        }
    }
    let report = evaluate_three_goal_benchmark(
        &observations,
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: 1_000,
            seed: 9,
        },
    )
    .unwrap();
    let headline = report
        .contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c0")
        .unwrap();
    assert_eq!(headline.quality.status, GoalStatus::Regressed);
    assert_eq!(headline.tokens.status, GoalStatus::Regressed);
    assert_eq!(headline.speed.status, GoalStatus::Regressed);
    assert!(report.statement.contains("quality=regressed"));
}
