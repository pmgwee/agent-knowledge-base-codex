use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use super::matrix::DeterministicRng;
use super::{BenchmarkPair, QualityEstimate, TokenEstimate};
use super::{PassRateEstimate, RatioOfSumsEstimate};

#[derive(Clone, Debug)]
pub(crate) struct MetricPair {
    pub task_id: String,
    pub baseline: u64,
    pub treatment: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct OutcomePair {
    pub task_id: String,
    pub baseline_passed: bool,
    pub treatment_passed: bool,
    pub treatment_critical_regression: bool,
    pub historical: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RatioBootstrap {
    pub samples: usize,
    pub baseline_total: u64,
    pub treatment_total: u64,
    pub point: f64,
    pub distribution: Vec<f64>,
    pub resamples: u32,
    pub seed: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DifferenceBootstrap {
    pub samples: usize,
    pub baseline_pass: usize,
    pub treatment_pass: usize,
    pub point: f64,
    pub distribution: Vec<f64>,
    pub critical_regressions: usize,
}

pub(crate) fn ratio_bootstrap(
    pairs: &[MetricPair],
    resamples: u32,
    seed: u64,
) -> Result<RatioBootstrap> {
    ensure!(!pairs.is_empty(), "no matched metric pairs");
    ensure!(resamples > 0, "bootstrap needs at least one resample");
    ensure!(
        pairs.iter().all(|pair| pair.baseline > 0),
        "baseline total is zero"
    );
    let (baseline_total, treatment_total) = metric_sums(pairs.iter());
    let point = savings(baseline_total, treatment_total);
    let mut by_task: BTreeMap<&str, Vec<&MetricPair>> = BTreeMap::new();
    for pair in pairs {
        by_task.entry(&pair.task_id).or_default().push(pair);
    }
    let clusters = by_task.into_values().collect::<Vec<_>>();
    let mut rng = DeterministicRng::new(seed);
    let mut distribution = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        let mut baseline = 0_u64;
        let mut treatment = 0_u64;
        for _ in 0..clusters.len() {
            let (cluster_baseline, cluster_treatment) =
                metric_sums(clusters[rng.index(clusters.len())].iter().copied());
            baseline = baseline.saturating_add(cluster_baseline);
            treatment = treatment.saturating_add(cluster_treatment);
        }
        distribution.push(savings(baseline, treatment));
    }
    distribution.sort_by(f64::total_cmp);
    Ok(RatioBootstrap {
        samples: pairs.len(),
        baseline_total,
        treatment_total,
        point,
        distribution,
        resamples,
        seed,
    })
}

pub(crate) fn ratio_estimate(bootstrap: &RatioBootstrap, alpha: f64) -> RatioOfSumsEstimate {
    let low = percentile(&bootstrap.distribution, alpha / 2.0);
    let high = percentile(&bootstrap.distribution, 1.0 - alpha / 2.0);
    RatioOfSumsEstimate {
        samples: bootstrap.samples,
        baseline_total: bootstrap.baseline_total,
        treatment_total: bootstrap.treatment_total,
        absolute_reduction: i128::from(bootstrap.baseline_total)
            .saturating_sub(i128::from(bootstrap.treatment_total))
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        reduction_fraction: bootstrap.point,
        reduction_percent: bootstrap.point * 100.0,
        confidence_low: low,
        confidence_high: high,
        confidence_low_percent: low * 100.0,
        confidence_high_percent: high * 100.0,
        bootstrap_resamples: bootstrap.resamples,
        bootstrap_seed: bootstrap.seed,
    }
}

pub(crate) fn difference_bootstrap(
    pairs: &[OutcomePair],
    resamples: u32,
    seed: u64,
) -> Result<DifferenceBootstrap> {
    ensure!(!pairs.is_empty(), "no matched outcome pairs");
    ensure!(resamples > 0, "bootstrap needs at least one resample");
    let mut by_task: BTreeMap<&str, Vec<&OutcomePair>> = BTreeMap::new();
    for pair in pairs {
        by_task.entry(&pair.task_id).or_default().push(pair);
    }
    let clusters = by_task.into_values().collect::<Vec<_>>();
    let mut rng = DeterministicRng::new(seed);
    let mut distribution = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        let mut baseline = 0_usize;
        let mut treatment = 0_usize;
        let mut total = 0_usize;
        for _ in 0..clusters.len() {
            for pair in &clusters[rng.index(clusters.len())] {
                baseline += usize::from(pair.baseline_passed);
                treatment += usize::from(pair.treatment_passed);
                total += 1;
            }
        }
        distribution.push(rate_difference(baseline, treatment, total));
    }
    distribution.sort_by(f64::total_cmp);
    let baseline_pass = pairs.iter().filter(|pair| pair.baseline_passed).count();
    let treatment_pass = pairs.iter().filter(|pair| pair.treatment_passed).count();
    Ok(DifferenceBootstrap {
        samples: pairs.len(),
        baseline_pass,
        treatment_pass,
        point: rate_difference(baseline_pass, treatment_pass, pairs.len()),
        distribution,
        critical_regressions: pairs
            .iter()
            .filter(|pair| pair.treatment_critical_regression)
            .count(),
    })
}

pub(crate) fn pass_rate_estimate(
    overall: &DifferenceBootstrap,
    historical: Option<&DifferenceBootstrap>,
    alpha: f64,
) -> PassRateEstimate {
    PassRateEstimate {
        samples: overall.samples,
        baseline_pass_fraction: overall.baseline_pass as f64 / overall.samples as f64,
        treatment_pass_fraction: overall.treatment_pass as f64 / overall.samples as f64,
        difference: overall.point,
        difference_percentage_points: overall.point * 100.0,
        confidence_low: percentile(&overall.distribution, alpha),
        confidence_high: percentile(&overall.distribution, 1.0 - alpha),
        historical_difference: historical.map(|estimate| estimate.point),
        historical_confidence_low: historical
            .map(|estimate| percentile(&estimate.distribution, alpha)),
        critical_regressions: overall.critical_regressions,
    }
}

/// Holm step-down alpha allocated to each preregistered contrast.
pub(crate) fn holm_alphas(distributions: &[Vec<f64>], family_alpha: f64) -> Vec<f64> {
    if distributions.is_empty() {
        return Vec::new();
    }
    let mut ranked = distributions
        .iter()
        .enumerate()
        .map(|(index, distribution)| (index, two_sided_zero_p(distribution)))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
    let mut alphas = vec![family_alpha; distributions.len()];
    for (rank, (index, _)) in ranked.into_iter().enumerate() {
        alphas[index] = family_alpha / (distributions.len() - rank) as f64;
    }
    alphas
}

fn metric_sums<'a>(pairs: impl Iterator<Item = &'a MetricPair>) -> (u64, u64) {
    pairs.fold((0_u64, 0_u64), |(baseline, treatment), pair| {
        (
            baseline.saturating_add(pair.baseline),
            treatment.saturating_add(pair.treatment),
        )
    })
}

fn two_sided_zero_p(distribution: &[f64]) -> f64 {
    let at_or_below = distribution.iter().filter(|value| **value <= 0.0).count() as f64;
    let at_or_above = distribution.iter().filter(|value| **value >= 0.0).count() as f64;
    (2.0 * at_or_below.min(at_or_above) / distribution.len() as f64).min(1.0)
}

pub fn clustered_estimate(
    pairs: &[BenchmarkPair],
    resamples: u32,
    seed: u64,
) -> Result<TokenEstimate> {
    ensure!(!pairs.is_empty(), "no matched token pairs");
    ensure!(resamples > 0, "bootstrap needs at least one resample");
    ensure!(
        pairs.iter().all(|pair| pair.control_tokens > 0),
        "control token total is zero"
    );
    let (control, treatment) = token_sums(pairs);
    let point = savings(control, treatment);
    let clusters = clusters(pairs);
    let mut rng = DeterministicRng::new(seed);
    let mut distribution = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        let mut sampled_control = 0u64;
        let mut sampled_treatment = 0u64;
        for _ in 0..clusters.len() {
            let selected = &clusters[rng.index(clusters.len())];
            let (cluster_control, cluster_treatment) = token_sums(selected);
            sampled_control = sampled_control.saturating_add(cluster_control);
            sampled_treatment = sampled_treatment.saturating_add(cluster_treatment);
        }
        distribution.push(savings(sampled_control, sampled_treatment));
    }
    distribution.sort_by(f64::total_cmp);
    Ok(TokenEstimate {
        pairs: pairs.len(),
        control_tokens: control,
        treatment_tokens: treatment,
        saved_tokens: i128::from(control)
            .saturating_sub(i128::from(treatment))
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        savings_fraction: point,
        confidence_low: percentile(&distribution, 0.025),
        confidence_high: percentile(&distribution, 0.975),
        bootstrap_resamples: resamples,
        bootstrap_seed: seed,
    })
}

pub fn quality_estimate(
    pairs: &[BenchmarkPair],
    resamples: u32,
    seed: u64,
) -> Result<QualityEstimate> {
    ensure!(!pairs.is_empty(), "no matched quality pairs");
    let clusters = clusters(pairs);
    let mut rng = DeterministicRng::new(seed ^ 0x5155_414C_4954_5901);
    let mut distribution = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        let mut control_success = 0usize;
        let mut treatment_success = 0usize;
        let mut total = 0usize;
        for _ in 0..clusters.len() {
            let selected = &clusters[rng.index(clusters.len())];
            for pair in selected {
                control_success += usize::from(pair.control_grade.is_success());
                treatment_success += usize::from(pair.treatment_grade.is_success());
                total += 1;
            }
        }
        distribution.push(rate_difference(control_success, treatment_success, total));
    }
    distribution.sort_by(f64::total_cmp);
    let control_success = pairs
        .iter()
        .filter(|pair| pair.control_grade.is_success())
        .count();
    let treatment_success = pairs
        .iter()
        .filter(|pair| pair.treatment_grade.is_success())
        .count();
    Ok(QualityEstimate {
        pairs: pairs.len(),
        control_success_fraction: control_success as f64 / pairs.len() as f64,
        treatment_success_fraction: treatment_success as f64 / pairs.len() as f64,
        difference: rate_difference(control_success, treatment_success, pairs.len()),
        one_sided_confidence_low: percentile(&distribution, 0.05),
        critical_regressions: pairs
            .iter()
            .filter(|pair| pair.treatment_critical_regression)
            .count(),
    })
}

fn clusters(pairs: &[BenchmarkPair]) -> Vec<Vec<&BenchmarkPair>> {
    let mut by_task: BTreeMap<&str, Vec<&BenchmarkPair>> = BTreeMap::new();
    for pair in pairs {
        by_task.entry(pair.task_id.as_str()).or_default().push(pair);
    }
    by_task.into_values().collect()
}

fn token_sums(pairs: &[impl std::borrow::Borrow<BenchmarkPair>]) -> (u64, u64) {
    pairs
        .iter()
        .fold((0u64, 0u64), |(control, treatment), pair| {
            let pair = pair.borrow();
            (
                control.saturating_add(pair.control_tokens),
                treatment.saturating_add(pair.treatment_tokens),
            )
        })
}

fn savings(control: u64, treatment: u64) -> f64 {
    1.0 - treatment as f64 / control as f64
}

fn rate_difference(control: usize, treatment: usize, total: usize) -> f64 {
    treatment as f64 / total as f64 - control as f64 / total as f64
}

fn percentile(sorted: &[f64], probability: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * probability).round() as usize;
    sorted[index]
}
