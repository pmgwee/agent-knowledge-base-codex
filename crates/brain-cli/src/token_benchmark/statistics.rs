use std::collections::BTreeMap;

use anyhow::{Result, ensure};

use super::matrix::DeterministicRng;
use super::{BenchmarkPair, QualityEstimate, TokenEstimate};

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
