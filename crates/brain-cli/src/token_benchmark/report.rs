use anyhow::{Result, ensure};

use super::{
    BenchmarkHarness, BenchmarkPair, BenchmarkStatus, HarnessBenchmarkReport, TokenBenchmarkReport,
    ValidityCheck, clustered_estimate, quality_estimate,
};

const QUALITY_MARGIN: f64 = -0.02;

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
        schema_version: 1,
        status,
        statement,
        minimum_pairs_per_harness,
        overall,
        overall_quality,
        harnesses,
        validity_checks: validity_checks.to_vec(),
    })
}

const fn harness_seed(harness: BenchmarkHarness) -> u64 {
    match harness {
        BenchmarkHarness::ClaudeCode => 0x434C_4155_4445,
        BenchmarkHarness::Codex => 0x434F_4445_5800,
    }
}
