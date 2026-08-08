use std::path::PathBuf;

use brain_cli::{LongMemEvalOptions, LongMemEvalReport, run_longmemeval};

/// Retrieval quality on the public LongMemEval-S benchmark.
///
/// Ignored by default because it needs a 264 MB dataset that is not vendored, and because a full
/// run ingests roughly a quarter of a million turns. Fetch it first:
///
/// ```text
/// curl -sL -o ~/AgentBrainBench/longmemeval_s.json \
///   https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json
/// cargo test -p brain-cli --test longmemeval -- --ignored --nocapture
/// ```
///
/// Environment switches, all optional:
///
/// | Variable | Effect |
/// |---|---|
/// | `LONGMEMEVAL_LIMIT` | Score only the first N matching instances |
/// | `LONGMEMEVAL_TYPES` | Comma-separated question types to score |
/// | `LONGMEMEVAL_BRAIN_HOME` | Embed events and fuse the vector channel in |
/// | `LONGMEMEVAL_DIVERSIFY` | `1` to cap results per session |
/// | `LONGMEMEVAL_RERANK` | `1` to re-rank the head with the cross-encoder (needs the brain home) |
///
/// There is no pass/fail threshold here on purpose. The number is the deliverable, and a gate
/// asserting it stays above whatever it happened to measure first would be a gate that only ever
/// encodes the status quo. It prints, and a human decides whether the number is good.
#[test]
#[ignore = "needs the LongMemEval-S dataset; see the doc comment"]
fn longmemeval_s_retrieval_quality() {
    let dataset = dataset_path();
    if !dataset.is_file() {
        panic!(
            "LongMemEval-S not found at {}. See this test's doc comment for the download.",
            dataset.display()
        );
    }
    let temp = tempfile::tempdir().expect("workspace");
    let limit = std::env::var("LONGMEMEVAL_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    let options = LongMemEvalOptions {
        question_types: std::env::var("LONGMEMEVAL_TYPES")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        brain_home: std::env::var("LONGMEMEVAL_BRAIN_HOME")
            .ok()
            .map(PathBuf::from),
        diversify_sessions: std::env::var("LONGMEMEVAL_DIVERSIFY").is_ok_and(|value| value == "1"),
        rerank: std::env::var("LONGMEMEVAL_RERANK").is_ok_and(|value| value == "1"),
    };

    let report = run_longmemeval(&dataset, temp.path(), limit, &options).expect("run benchmark");
    report_to_stdout(&report);

    assert!(
        report.instances_scored > 0,
        "a run that scored nothing is not a result"
    );
}

fn report_to_stdout(report: &LongMemEvalReport) {
    println!("\n=== LongMemEval-S ===");
    println!("configuration    : {}", report.configuration);
    println!("instances scored : {}", report.instances_scored);
    println!(
        "corpus ingested  : {} sessions, {} turns",
        report.sessions_ingested, report.turns_ingested
    );
    if report.vectors_built > 0 {
        println!("vectors built    : {}", report.vectors_built);
    }
    for (k, recall) in &report.recall_at {
        println!("R@{k:<14} : {:.1}%", recall * 100.0);
    }
    println!("MRR              : {:.3}", report.mean_reciprocal_rank);
    println!("elapsed          : {:.1}s", report.elapsed_seconds);
    println!("\n-- R@5 by question type --");
    for (kind, recall) in &report.recall_at_5_by_type {
        println!("  {kind:<28} {:.1}%", recall * 100.0);
    }
    println!();
}

fn dataset_path() -> PathBuf {
    std::env::var("LONGMEMEVAL_DATASET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();
            PathBuf::from(home)
                .join("AgentBrainBench")
                .join("longmemeval_s.json")
        })
}
