use brain_cli::{
    BenchmarkProfile, benchmark_corpus, benchmark_report_dir, corpus_hashes,
    preserve_benchmark_report,
};

#[test]
fn equal_seed_produces_equal_event_and_answer_hashes() {
    let first = corpus_hashes(BenchmarkProfile::Smoke, 42);
    let second = corpus_hashes(BenchmarkProfile::Smoke, 42);
    assert_eq!(first, second);
}

#[test]
fn smoke_corpus_passes_the_same_correctness_contract() {
    let temp = tempfile::tempdir().expect("temp");
    let corpus_root = temp.path().join("smoke");
    let report = benchmark_corpus(corpus_root.clone(), BenchmarkProfile::Smoke, 42)
        .expect("smoke benchmark");
    let saved = preserve_benchmark_report(&corpus_root, &benchmark_report_dir(), "smoke")
        .expect("preserve smoke report");
    println!("preserved report: {}", saved.display());
    assert!(report.passed, "{:#?}", report.failures);
}

#[test]
#[ignore = "production release gate: generates 12,000 sessions and 6,000,000 events"]
fn primary_production_corpus_passes_all_gates() {
    let temp = tempfile::tempdir().expect("temp");
    let corpus_root = temp.path().join("primary");
    let report = benchmark_corpus(corpus_root.clone(), BenchmarkProfile::Primary, 42)
        .expect("primary benchmark");
    // Preserve before asserting: a failed threshold is when the measurements matter most,
    // and the panic would otherwise take the corpus directory and the report with it.
    let saved = preserve_benchmark_report(&corpus_root, &benchmark_report_dir(), "primary")
        .expect("preserve primary report");
    println!("preserved report: {}", saved.display());
    println!("{}", serde_json::to_string_pretty(&report).expect("report"));
    assert!(report.passed, "{:#?}", report.failures);
}
