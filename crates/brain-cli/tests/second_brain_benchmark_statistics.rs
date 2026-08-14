use brain_cli::{
    BenchmarkCondition, BenchmarkHarness, BenchmarkObservation, BenchmarkTaskStratum, GoalStatus,
    GradeOutcome, ThreeGoalEvaluationOptions, classify_quality_safety, classify_reduction,
    evaluate_three_goal_benchmark,
};

fn observation(
    task: &str,
    harness: BenchmarkHarness,
    condition: BenchmarkCondition,
    tokens: Option<u64>,
    elapsed_ms: Option<u64>,
) -> BenchmarkObservation {
    BenchmarkObservation {
        task_id: task.to_owned(),
        harness,
        repeat: 1,
        stratum: BenchmarkTaskStratum::HistoricalRecall,
        condition,
        native_tokens: tokens,
        elapsed_ms,
        grade: GradeOutcome::Pass,
        critical_regression: false,
        timed_out: false,
        harness_failed: false,
        brain_mcp_calls: u32::from(condition == BenchmarkCondition::C4),
    }
}

fn complete_block() -> Vec<BenchmarkObservation> {
    let mut observations = Vec::new();
    for harness in BenchmarkHarness::ALL {
        for (condition, tokens, elapsed) in [
            (BenchmarkCondition::C0, 100, 100),
            (BenchmarkCondition::C1, 95, 95),
            (BenchmarkCondition::C2, 90, 90),
            (BenchmarkCondition::C3, 85, 85),
            (BenchmarkCondition::C4, 80, 70),
        ] {
            observations.push(observation(
                "task-a",
                harness,
                condition,
                Some(tokens),
                Some(elapsed),
            ));
        }
    }
    observations
}

#[test]
fn all_six_contrasts_use_ratio_of_sums_and_keep_three_verdicts_separate() {
    let report = evaluate_three_goal_benchmark(
        &complete_block(),
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: 10_000,
            seed: 42,
        },
    )
    .unwrap();
    assert_eq!(report.contrasts.len(), 6);
    let headline = report
        .contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c0")
        .unwrap();
    assert_eq!(headline.tokens.status, GoalStatus::Proven);
    assert_eq!(headline.speed.status, GoalStatus::Proven);
    assert_eq!(headline.quality.status, GoalStatus::Inconclusive);
    assert!((headline.tokens.estimate.as_ref().unwrap().reduction_percent - 20.0).abs() < 1e-9);
    assert!((headline.speed.estimate.as_ref().unwrap().reduction_percent - 30.0).abs() < 1e-9);
    assert_eq!(headline.mcp_adoption.assigned, 2);
    assert_eq!(headline.mcp_adoption.called, 2);
    assert!(report.statement.contains("quality=inconclusive"));
}

#[test]
fn missing_blocks_and_invalid_native_usage_invalidate_only_the_affected_goal() {
    let mut observations = complete_block();
    observations.retain(|item| {
        !(item.harness == BenchmarkHarness::Codex && item.condition == BenchmarkCondition::C3)
    });
    observations
        .iter_mut()
        .find(|item| {
            item.harness == BenchmarkHarness::ClaudeCode && item.condition == BenchmarkCondition::C4
        })
        .unwrap()
        .native_tokens = None;
    let report = evaluate_three_goal_benchmark(
        &observations,
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: 1_000,
            seed: 7,
        },
    )
    .unwrap();
    let c4_c3 = report
        .contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c3")
        .unwrap();
    assert_eq!(c4_c3.tokens.status, GoalStatus::Invalid);
    assert_eq!(c4_c3.speed.status, GoalStatus::Invalid);
    let c4_c0 = report
        .contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c0")
        .unwrap();
    assert_eq!(c4_c0.tokens.status, GoalStatus::Invalid);
    assert_ne!(c4_c0.speed.status, GoalStatus::Invalid);
}

#[test]
fn conflicting_per_harness_directions_cannot_be_called_a_saving() {
    let mut observations = complete_block();
    observations
        .iter_mut()
        .find(|item| {
            item.harness == BenchmarkHarness::Codex && item.condition == BenchmarkCondition::C4
        })
        .unwrap()
        .native_tokens = Some(120);
    let report = evaluate_three_goal_benchmark(
        &observations,
        ThreeGoalEvaluationOptions {
            bootstrap_resamples: 1_000,
            seed: 8,
        },
    )
    .unwrap();
    let headline = report
        .contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c0")
        .unwrap();
    assert_eq!(headline.tokens.status, GoalStatus::Inconclusive);
}

#[test]
fn preregistered_gate_boundaries_are_literal() {
    assert_eq!(
        classify_reduction(0.10, 0.001, &[0.001, 0.002], true, false),
        GoalStatus::Proven
    );
    assert_eq!(
        classify_reduction(0.10, 0.0, &[0.001, 0.002], true, false),
        GoalStatus::Inconclusive
    );
    assert_eq!(
        classify_reduction(0.0, -0.01, &[-0.01, 0.01], true, false),
        GoalStatus::Inconclusive
    );
    assert_eq!(
        classify_quality_safety(-0.02, -0.02, false),
        GoalStatus::Proven
    );
    assert_eq!(
        classify_quality_safety(-0.019, -0.019, false),
        GoalStatus::Proven
    );
}
