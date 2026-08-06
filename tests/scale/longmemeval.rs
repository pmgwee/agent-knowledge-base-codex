use std::path::PathBuf;

use brain_cli::run_longmemeval;

/// Retrieval quality on the public LongMemEval-S benchmark.
///
/// Ignored by default because it needs a 264 MB dataset that is not vendored, and because a
/// full run ingests roughly a quarter of a million turns. Fetch it first:
///
/// ```text
/// curl -sL -o ~/AgentBrainBench/longmemeval_s.json \
///   https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json
/// cargo test -p brain-cli --test longmemeval -- --ignored --nocapture
/// ```
///
/// There is no pass/fail threshold here on purpose. The number is the deliverable, and a gate
/// asserting it stays above whatever it happened to measure first would be a gate that only
/// ever encodes the status quo. It prints, and a human decides whether the number is good.
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

    let report = run_longmemeval(&dataset, temp.path(), limit).expect("run benchmark");

    println!("\n=== LongMemEval-S — ledger FTS5 (BM25), no vector, no rerank ===");
    println!("instances scored : {}", report.instances_scored);
    println!(
        "corpus ingested  : {} sessions, {} turns",
        report.sessions_ingested, report.turns_ingested
    );
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

    assert!(
        report.instances_scored > 0,
        "a run that scored nothing is not a result"
    );
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
