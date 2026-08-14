use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use super::statistics::{
    DifferenceBootstrap, MetricPair, OutcomePair, RatioBootstrap, difference_bootstrap,
    holm_alphas, pass_rate_estimate, ratio_bootstrap, ratio_estimate,
};
use super::{
    BenchmarkCondition, BenchmarkContrast, BenchmarkHarness, BenchmarkObservation, BenchmarkPair,
    BenchmarkStatus, BenchmarkTaskStratum, GoalEstimate, GoalStatus, HarnessBenchmarkReport,
    IntentionToTreatDiagnostic, McpAdoptionDiagnostic, PRIMARY_CONTRASTS, RetrievalContrastMetrics,
    ThreeGoalBenchmarkReport, ThreeGoalContrastReport, ThreeGoalEvaluationOptions,
    TokenBenchmarkReport, ValidityCheck, clustered_estimate, quality_estimate,
};

const QUALITY_MARGIN: f64 = -0.02;

#[derive(Default)]
struct PreparedContrast {
    token_pairs: Vec<MetricPair>,
    time_pairs: Vec<MetricPair>,
    outcome_pairs: Vec<OutcomePair>,
    historical_pairs: Vec<OutcomePair>,
    token_by_harness: BTreeMap<BenchmarkHarness, Vec<MetricPair>>,
    time_by_harness: BTreeMap<BenchmarkHarness, Vec<MetricPair>>,
    outcome_by_harness: BTreeMap<BenchmarkHarness, Vec<OutcomePair>>,
    historical_by_harness: BTreeMap<BenchmarkHarness, Vec<OutcomePair>>,
    missing_pairs: usize,
    complete_pairs: usize,
    invalid_token_samples: usize,
    invalid_time_samples: usize,
    timed_out_samples: usize,
    harness_failure_samples: usize,
    baseline_failures: usize,
    treatment_failures: usize,
    mcp_assigned: u64,
    mcp_called: u64,
}

struct BootstrappedContrast {
    prepared: PreparedContrast,
    tokens: Option<RatioBootstrap>,
    speed: Option<RatioBootstrap>,
    quality: Option<DifferenceBootstrap>,
    historical_quality: Option<DifferenceBootstrap>,
    token_harnesses: BTreeMap<BenchmarkHarness, RatioBootstrap>,
    speed_harnesses: BTreeMap<BenchmarkHarness, RatioBootstrap>,
    quality_harnesses: BTreeMap<BenchmarkHarness, DifferenceBootstrap>,
    historical_quality_harnesses: BTreeMap<BenchmarkHarness, DifferenceBootstrap>,
}

#[derive(Clone, Copy)]
struct AdjustedAlphas {
    token: f64,
    speed: f64,
    quality: f64,
}

pub fn evaluate_three_goal_benchmark(
    observations: &[BenchmarkObservation],
    options: ThreeGoalEvaluationOptions,
) -> Result<ThreeGoalBenchmarkReport> {
    evaluate_three_goal_benchmark_with_retrieval(observations, options, &BTreeMap::new())
}

pub fn evaluate_three_goal_benchmark_with_retrieval(
    observations: &[BenchmarkObservation],
    options: ThreeGoalEvaluationOptions,
    retrieval: &BTreeMap<String, RetrievalContrastMetrics>,
) -> Result<ThreeGoalBenchmarkReport> {
    ensure!(
        options.bootstrap_resamples > 0,
        "bootstrap needs at least one resample"
    );
    if observations.is_empty() {
        return Ok(not_run_report(options));
    }
    let mut blocks: BTreeMap<
        (&str, BenchmarkHarness, u32),
        BTreeMap<BenchmarkCondition, &BenchmarkObservation>,
    > = BTreeMap::new();
    for observation in observations {
        let block = blocks
            .entry((
                &observation.task_id,
                observation.harness,
                observation.repeat,
            ))
            .or_default();
        ensure!(
            block.insert(observation.condition, observation).is_none(),
            "duplicate condition in benchmark block"
        );
    }

    let mut bootstrapped = Vec::with_capacity(PRIMARY_CONTRASTS.len());
    for (contrast_index, contrast) in PRIMARY_CONTRASTS.iter().enumerate() {
        let prepared = prepare_contrast(&blocks, contrast.baseline, contrast.treatment);
        let contrast_seed = options.seed ^ ((contrast_index as u64 + 1) * 0x9E37_79B9);
        bootstrapped.push(bootstrap_prepared(
            prepared,
            options.bootstrap_resamples,
            contrast_seed,
        )?);
    }

    let token_alphas = family_alphas(&bootstrapped, |contrast| {
        contrast.tokens.as_ref().map(|value| &value.distribution)
    });
    let speed_alphas = family_alphas(&bootstrapped, |contrast| {
        contrast.speed.as_ref().map(|value| &value.distribution)
    });
    let quality_alphas = family_alphas(&bootstrapped, |contrast| {
        contrast.quality.as_ref().map(|value| &value.distribution)
    });

    let mut contrasts = Vec::with_capacity(PRIMARY_CONTRASTS.len());
    for (index, (definition, raw)) in PRIMARY_CONTRASTS
        .iter()
        .zip(bootstrapped.into_iter())
        .enumerate()
    {
        contrasts.push(finalize_contrast(
            *definition,
            raw,
            AdjustedAlphas {
                token: token_alphas[index],
                speed: speed_alphas[index],
                quality: quality_alphas[index],
            },
            retrieval.get(definition.id).cloned(),
        ));
    }
    let headline = contrasts
        .iter()
        .find(|contrast| contrast.contrast_id == "c4_vs_c0")
        .expect("primary contrast is registered");
    let statement = format!(
        "C4 vs C0: tokens={}, speed={}, quality={}. Each goal is evaluated independently; no passing goal hides another goal's failure or uncertainty.",
        headline.tokens.status.as_str(),
        headline.speed.status.as_str(),
        headline.quality.status.as_str()
    );
    Ok(ThreeGoalBenchmarkReport {
        schema_version: 3,
        bootstrap_resamples: options.bootstrap_resamples,
        bootstrap_seed: options.seed,
        statement,
        contrasts,
    })
}

pub fn classify_reduction(
    point: f64,
    confidence_low: f64,
    harness_confidence_lows: &[f64],
    quality_safe: bool,
    critical_regression: bool,
) -> GoalStatus {
    if critical_regression || !quality_safe {
        return GoalStatus::Regressed;
    }
    if point >= 0.10
        && confidence_low > 0.0
        && !harness_confidence_lows.is_empty()
        && harness_confidence_lows.iter().all(|low| *low > 0.0)
    {
        GoalStatus::Proven
    } else if point < 0.0 {
        GoalStatus::Regressed
    } else {
        GoalStatus::Inconclusive
    }
}

pub fn classify_quality_safety(
    point: f64,
    one_sided_confidence_low: f64,
    critical_regression: bool,
) -> GoalStatus {
    if critical_regression || point < QUALITY_MARGIN || one_sided_confidence_low < QUALITY_MARGIN {
        GoalStatus::Regressed
    } else {
        GoalStatus::Proven
    }
}

fn prepare_contrast(
    blocks: &BTreeMap<
        (&str, BenchmarkHarness, u32),
        BTreeMap<BenchmarkCondition, &BenchmarkObservation>,
    >,
    baseline_condition: BenchmarkCondition,
    treatment_condition: BenchmarkCondition,
) -> PreparedContrast {
    let mut prepared = PreparedContrast::default();
    for ((task_id, harness, _), block) in blocks {
        let (Some(baseline), Some(treatment)) = (
            block.get(&baseline_condition).copied(),
            block.get(&treatment_condition).copied(),
        ) else {
            prepared.missing_pairs += 1;
            continue;
        };
        prepared.complete_pairs += 1;
        prepared.invalid_token_samples += usize::from(baseline.native_tokens.is_none())
            + usize::from(treatment.native_tokens.is_none());
        prepared.invalid_time_samples += usize::from(baseline.elapsed_ms.is_none())
            + usize::from(treatment.elapsed_ms.is_none());
        prepared.timed_out_samples +=
            usize::from(baseline.timed_out) + usize::from(treatment.timed_out);
        prepared.harness_failure_samples +=
            usize::from(baseline.harness_failed) + usize::from(treatment.harness_failed);
        let baseline_failed = baseline.timed_out || baseline.harness_failed;
        let treatment_failed = treatment.timed_out || treatment.harness_failed;
        prepared.baseline_failures += usize::from(baseline_failed);
        prepared.treatment_failures += usize::from(treatment_failed);
        if let (Some(baseline_tokens), Some(treatment_tokens)) =
            (baseline.native_tokens, treatment.native_tokens)
        {
            let pair = MetricPair {
                task_id: (*task_id).to_owned(),
                baseline: baseline_tokens,
                treatment: treatment_tokens,
            };
            prepared.token_pairs.push(pair.clone());
            prepared
                .token_by_harness
                .entry(*harness)
                .or_default()
                .push(pair);
        }
        if let (Some(baseline_ms), Some(treatment_ms)) = (baseline.elapsed_ms, treatment.elapsed_ms)
        {
            let pair = MetricPair {
                task_id: (*task_id).to_owned(),
                baseline: baseline_ms,
                treatment: treatment_ms,
            };
            prepared.time_pairs.push(pair.clone());
            prepared
                .time_by_harness
                .entry(*harness)
                .or_default()
                .push(pair);
        }
        let outcome = OutcomePair {
            task_id: (*task_id).to_owned(),
            baseline_passed: baseline.grade.is_success() && !baseline_failed,
            treatment_passed: treatment.grade.is_success() && !treatment_failed,
            treatment_critical_regression: treatment.critical_regression,
            historical: baseline.stratum == BenchmarkTaskStratum::HistoricalRecall,
        };
        prepared.outcome_pairs.push(outcome.clone());
        prepared
            .outcome_by_harness
            .entry(*harness)
            .or_default()
            .push(outcome.clone());
        if outcome.historical {
            prepared.historical_pairs.push(outcome.clone());
            prepared
                .historical_by_harness
                .entry(*harness)
                .or_default()
                .push(outcome);
        }
        if treatment_condition == BenchmarkCondition::C4 {
            prepared.mcp_assigned += 1;
            prepared.mcp_called += u64::from(treatment.brain_mcp_calls > 0);
        }
    }
    prepared
}

fn bootstrap_prepared(
    prepared: PreparedContrast,
    resamples: u32,
    seed: u64,
) -> Result<BootstrappedContrast> {
    let tokens = (!prepared.token_pairs.is_empty())
        .then(|| ratio_bootstrap(&prepared.token_pairs, resamples, seed ^ 0x544F_4B45))
        .transpose()?;
    let speed = (!prepared.time_pairs.is_empty())
        .then(|| ratio_bootstrap(&prepared.time_pairs, resamples, seed ^ 0x5449_4D45))
        .transpose()?;
    let quality = (!prepared.outcome_pairs.is_empty())
        .then(|| difference_bootstrap(&prepared.outcome_pairs, resamples, seed ^ 0x5155_414C))
        .transpose()?;
    let historical_quality = (!prepared.historical_pairs.is_empty())
        .then(|| difference_bootstrap(&prepared.historical_pairs, resamples, seed ^ 0x4849_5354))
        .transpose()?;
    let mut token_harnesses = BTreeMap::new();
    let mut speed_harnesses = BTreeMap::new();
    let mut quality_harnesses = BTreeMap::new();
    let mut historical_quality_harnesses = BTreeMap::new();
    for harness in BenchmarkHarness::ALL {
        if let Some(pairs) = prepared.token_by_harness.get(&harness) {
            token_harnesses.insert(
                harness,
                ratio_bootstrap(pairs, resamples, seed ^ harness_seed(harness))?,
            );
        }
        if let Some(pairs) = prepared.time_by_harness.get(&harness) {
            speed_harnesses.insert(
                harness,
                ratio_bootstrap(pairs, resamples, seed ^ harness_seed(harness) ^ 1)?,
            );
        }
        if let Some(pairs) = prepared.outcome_by_harness.get(&harness) {
            quality_harnesses.insert(
                harness,
                difference_bootstrap(pairs, resamples, seed ^ harness_seed(harness) ^ 2)?,
            );
        }
        if let Some(pairs) = prepared.historical_by_harness.get(&harness) {
            historical_quality_harnesses.insert(
                harness,
                difference_bootstrap(pairs, resamples, seed ^ harness_seed(harness) ^ 3)?,
            );
        }
    }
    Ok(BootstrappedContrast {
        prepared,
        tokens,
        speed,
        quality,
        historical_quality,
        token_harnesses,
        speed_harnesses,
        quality_harnesses,
        historical_quality_harnesses,
    })
}

fn family_alphas(
    contrasts: &[BootstrappedContrast],
    distribution: impl Fn(&BootstrappedContrast) -> Option<&Vec<f64>>,
) -> Vec<f64> {
    let available = contrasts
        .iter()
        .filter_map(&distribution)
        .cloned()
        .collect::<Vec<_>>();
    let adjusted = holm_alphas(&available, 0.05);
    let mut cursor = 0;
    contrasts
        .iter()
        .map(|contrast| {
            if distribution(contrast).is_some() {
                let alpha = adjusted[cursor];
                cursor += 1;
                alpha
            } else {
                0.05
            }
        })
        .collect()
}

fn finalize_contrast(
    definition: BenchmarkContrast,
    raw: BootstrappedContrast,
    alphas: AdjustedAlphas,
    retrieval: Option<RetrievalContrastMetrics>,
) -> ThreeGoalContrastReport {
    let missing = raw.prepared.missing_pairs > 0;
    let quality_estimate = raw.quality.as_ref().map(|overall| {
        pass_rate_estimate(overall, raw.historical_quality.as_ref(), alphas.quality)
    });
    let quality_harness_estimates = raw
        .quality_harnesses
        .iter()
        .map(|(harness, estimate)| {
            (
                *harness,
                pass_rate_estimate(
                    estimate,
                    raw.historical_quality_harnesses.get(harness),
                    alphas.quality,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let quality_safe = quality_estimate.as_ref().is_some_and(|estimate| {
        classify_quality_safety(
            estimate.difference,
            estimate.confidence_low,
            estimate.critical_regressions > 0,
        ) == GoalStatus::Proven
            && quality_harness_estimates.values().all(|harness| {
                classify_quality_safety(
                    harness.difference,
                    harness.confidence_low,
                    harness.critical_regressions > 0,
                ) == GoalStatus::Proven
            })
    });
    let quality_status = if missing || quality_estimate.is_none() {
        GoalStatus::Invalid
    } else if !quality_safe {
        GoalStatus::Regressed
    } else {
        let estimate = quality_estimate.as_ref().expect("checked");
        let historical_superior = estimate
            .historical_difference
            .zip(estimate.historical_confidence_low)
            .is_some_and(|(point, low)| point >= 0.05 && low > 0.0)
            && quality_harness_estimates.values().all(|harness| {
                harness
                    .historical_difference
                    .zip(harness.historical_confidence_low)
                    .is_some_and(|(point, low)| point >= 0.05 && low > 0.0)
            });
        let retrieval_passes = retrieval.as_ref().is_some_and(|metrics| {
            metrics.precision_at_5_percent >= 90.0
                && metrics.recall_at_5_percent >= 90.0
                && metrics.citation_precision_percent == 100.0
                && metrics.faithfulness_percent >= 98.0
                && metrics.cross_project_leakage_percent == 0.0
                && metrics.stale_contradiction_percent == 0.0
        });
        if historical_superior && retrieval_passes {
            GoalStatus::Proven
        } else {
            GoalStatus::Inconclusive
        }
    };

    let token_harness_estimates = raw
        .token_harnesses
        .iter()
        .map(|(harness, estimate)| (*harness, ratio_estimate(estimate, alphas.token)))
        .collect::<BTreeMap<_, _>>();
    let token_estimate = raw
        .tokens
        .as_ref()
        .map(|estimate| ratio_estimate(estimate, alphas.token));
    let token_status =
        if missing || raw.prepared.invalid_token_samples > 0 || token_estimate.is_none() {
            GoalStatus::Invalid
        } else {
            let estimate = token_estimate.as_ref().expect("checked");
            classify_reduction(
                estimate.reduction_fraction,
                estimate.confidence_low,
                &token_harness_estimates
                    .values()
                    .map(|harness| harness.confidence_low)
                    .collect::<Vec<_>>(),
                quality_safe,
                quality_estimate
                    .as_ref()
                    .is_some_and(|quality| quality.critical_regressions > 0),
            )
        };
    let speed_harness_estimates = raw
        .speed_harnesses
        .iter()
        .map(|(harness, estimate)| (*harness, ratio_estimate(estimate, alphas.speed)))
        .collect::<BTreeMap<_, _>>();
    let speed_estimate = raw
        .speed
        .as_ref()
        .map(|estimate| ratio_estimate(estimate, alphas.speed));
    let failure_rate_increase = if raw.prepared.complete_pairs == 0 {
        0.0
    } else {
        (raw.prepared.treatment_failures as f64 - raw.prepared.baseline_failures as f64)
            / raw.prepared.complete_pairs as f64
    };
    let speed_status =
        if missing || raw.prepared.invalid_time_samples > 0 || speed_estimate.is_none() {
            GoalStatus::Invalid
        } else if failure_rate_increase > 0.01 {
            GoalStatus::Regressed
        } else {
            let estimate = speed_estimate.as_ref().expect("checked");
            classify_reduction(
                estimate.reduction_fraction,
                estimate.confidence_low,
                &speed_harness_estimates
                    .values()
                    .map(|harness| harness.confidence_low)
                    .collect::<Vec<_>>(),
                quality_safe,
                quality_estimate
                    .as_ref()
                    .is_some_and(|quality| quality.critical_regressions > 0),
            )
        };

    let invalid_reasons = |kind: &str, invalid: usize| {
        let mut reasons = Vec::new();
        if missing {
            reasons.push(format!(
                "{} condition pairs are missing",
                raw.prepared.missing_pairs
            ));
        }
        if invalid > 0 {
            reasons.push(format!(
                "{invalid} {kind} samples lack a native measurement"
            ));
        }
        reasons
    };
    ThreeGoalContrastReport {
        contrast_id: definition.id.to_owned(),
        baseline: definition.baseline,
        treatment: definition.treatment,
        tokens: GoalEstimate {
            status: token_status,
            estimate: token_estimate,
            harness_estimates: token_harness_estimates,
            invalid_reasons: invalid_reasons("token", raw.prepared.invalid_token_samples),
        },
        speed: GoalEstimate {
            status: speed_status,
            estimate: speed_estimate,
            harness_estimates: speed_harness_estimates,
            invalid_reasons: invalid_reasons("time", raw.prepared.invalid_time_samples),
        },
        quality: GoalEstimate {
            status: quality_status,
            estimate: quality_estimate,
            harness_estimates: quality_harness_estimates,
            invalid_reasons: invalid_reasons("quality", 0),
        },
        retrieval,
        intention_to_treat: IntentionToTreatDiagnostic {
            complete_pairs: raw.prepared.complete_pairs,
            missing_pairs: raw.prepared.missing_pairs,
            invalid_token_samples: raw.prepared.invalid_token_samples,
            invalid_time_samples: raw.prepared.invalid_time_samples,
            timed_out_samples: raw.prepared.timed_out_samples,
            harness_failure_samples: raw.prepared.harness_failure_samples,
        },
        mcp_adoption: McpAdoptionDiagnostic {
            assigned: raw.prepared.mcp_assigned,
            called: raw.prepared.mcp_called,
            adoption_percent: (raw.prepared.mcp_assigned > 0)
                .then(|| raw.prepared.mcp_called as f64 * 100.0 / raw.prepared.mcp_assigned as f64),
        },
    }
}

fn not_run_report(options: ThreeGoalEvaluationOptions) -> ThreeGoalBenchmarkReport {
    let contrasts = PRIMARY_CONTRASTS
        .iter()
        .map(|contrast| ThreeGoalContrastReport {
            contrast_id: contrast.id.to_owned(),
            baseline: contrast.baseline,
            treatment: contrast.treatment,
            tokens: empty_goal(GoalStatus::NotRun),
            speed: empty_goal(GoalStatus::NotRun),
            quality: empty_goal(GoalStatus::NotRun),
            retrieval: None,
            intention_to_treat: IntentionToTreatDiagnostic {
                complete_pairs: 0,
                missing_pairs: 0,
                invalid_token_samples: 0,
                invalid_time_samples: 0,
                timed_out_samples: 0,
                harness_failure_samples: 0,
            },
            mcp_adoption: McpAdoptionDiagnostic {
                assigned: 0,
                called: 0,
                adoption_percent: None,
            },
        })
        .collect();
    ThreeGoalBenchmarkReport {
        schema_version: 3,
        bootstrap_resamples: options.bootstrap_resamples,
        bootstrap_seed: options.seed,
        statement: "Not measured: no benchmark observations were supplied.".to_owned(),
        contrasts,
    }
}

fn empty_goal<T>(status: GoalStatus) -> GoalEstimate<T> {
    GoalEstimate {
        status,
        estimate: None,
        harness_estimates: BTreeMap::new(),
        invalid_reasons: Vec::new(),
    }
}

pub fn evaluate_benchmark(
    pairs: &[BenchmarkPair],
    validity_checks: &[ValidityCheck],
    resamples: u32,
    seed: u64,
    minimum_pairs_per_harness: usize,
) -> Result<TokenBenchmarkReport> {
    ensure!(
        minimum_pairs_per_harness > 0,
        "minimum pairs must be positive"
    );
    let failed_check = validity_checks.iter().find(|check| !check.passed);
    let mut harnesses = Vec::new();
    for harness in BenchmarkHarness::ALL {
        let selected: Vec<_> = pairs
            .iter()
            .filter(|pair| pair.harness == harness)
            .cloned()
            .collect();
        if !selected.is_empty() {
            harnesses.push(HarnessBenchmarkReport {
                harness,
                tokens: clustered_estimate(&selected, resamples, seed ^ harness_seed(harness))?,
                quality: quality_estimate(&selected, resamples, seed ^ harness_seed(harness))?,
            });
        }
    }
    let balanced = harnesses.len() == 2
        && harnesses[0].tokens.pairs == harnesses[1].tokens.pairs
        && harnesses
            .iter()
            .all(|report| report.tokens.pairs >= minimum_pairs_per_harness);
    let overall = balanced
        .then(|| clustered_estimate(pairs, resamples, seed))
        .transpose()?;
    let overall_quality = balanced
        .then(|| quality_estimate(pairs, resamples, seed))
        .transpose()?;

    let quality_passes = balanced
        && harnesses.iter().all(|report| {
            report.quality.critical_regressions == 0
                && report.quality.one_sided_confidence_low >= QUALITY_MARGIN
        })
        && overall_quality.as_ref().is_some_and(|quality| {
            quality.critical_regressions == 0 && quality.one_sided_confidence_low >= QUALITY_MARGIN
        });
    let token_passes = balanced
        && harnesses
            .iter()
            .all(|report| report.tokens.confidence_low > 0.0)
        && overall
            .as_ref()
            .is_some_and(|estimate| estimate.confidence_low > 0.0);

    let (status, statement) = if let Some(check) = failed_check {
        (
            BenchmarkStatus::Invalid,
            format!(
                "This run cannot support a token-savings claim because {}: {}.",
                check.name, check.detail
            ),
        )
    } else if !balanced {
        (
            BenchmarkStatus::Invalid,
            "This run cannot support a token-savings claim because matched per-harness coverage is incomplete or unbalanced.".to_owned(),
        )
    } else if !quality_passes {
        let lower_usage_observed = overall
            .as_ref()
            .is_some_and(|estimate| estimate.savings_fraction > 0.0);
        (
            BenchmarkStatus::QualityBlocked,
            if lower_usage_observed {
                "Lower token usage was observed, but the quality gate failed. No token-savings claim is valid for this run.".to_owned()
            } else {
                "The quality gate failed. No token-savings claim is valid for this run.".to_owned()
            },
        )
    } else if !token_passes {
        (
            BenchmarkStatus::Inconclusive,
            "This benchmark did not demonstrate a statistically reliable token saving. The measured interval includes zero; no savings percentage is claimed.".to_owned(),
        )
    } else {
        let combined = overall.as_ref().expect("balanced estimate");
        let claude = harnesses
            .iter()
            .find(|report| report.harness == BenchmarkHarness::ClaudeCode)
            .expect("Claude report");
        let codex = harnesses
            .iter()
            .find(|report| report.harness == BenchmarkHarness::Codex)
            .expect("Codex report");
        (
            BenchmarkStatus::Proven,
            format!(
                "Using exact provider-reported token counts across {} matched Claude Code and Codex pairs, Second Brain reduced total tokens per matched task by {:.1}% overall (95% CI {:.1}-{:.1}%). Claude Code saved {:.1}% and Codex saved {:.1}%. The preregistered quality gate passed with zero critical regressions.",
                combined.pairs,
                combined.savings_fraction * 100.0,
                combined.confidence_low * 100.0,
                combined.confidence_high * 100.0,
                claude.tokens.savings_fraction * 100.0,
                codex.tokens.savings_fraction * 100.0,
            ),
        )
    };

    Ok(TokenBenchmarkReport {
        schema_version: 2,
        status,
        statement,
        minimum_pairs_per_harness,
        overall,
        overall_quality,
        harnesses,
        validity_checks: validity_checks.to_vec(),
        three_goal: None,
    })
}

const fn harness_seed(harness: BenchmarkHarness) -> u64 {
    match harness {
        BenchmarkHarness::ClaudeCode => 0x434C_4155_4445,
        BenchmarkHarness::Codex => 0x434F_4445_5800,
    }
}
