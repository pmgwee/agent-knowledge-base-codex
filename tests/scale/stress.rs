use brain_cli::{
    BenchmarkProfile, benchmark_corpus, benchmark_report_dir, preserve_benchmark_report,
};

#[test]
#[ignore = "overnight release-candidate gate: 120,000 sessions and 60,000,000 events"]
fn stress_corpus_keeps_startup_query_and_memory_bounds() {
    let temp = tempfile::tempdir().expect("temp");
    let corpus_root = temp.path().join("stress");
    let report = benchmark_corpus(corpus_root.clone(), BenchmarkProfile::Stress, 42)
        .expect("stress benchmark");
    // Preserve before asserting: a failed threshold is when the measurements matter most,
    // and the panic would otherwise take the corpus directory and the report with it.
    let saved = preserve_benchmark_report(&corpus_root, &benchmark_report_dir(), "stress")
        .expect("preserve stress report");
    println!("preserved report: {}", saved.display());
    println!("{}", serde_json::to_string_pretty(&report).expect("report"));
    assert!(report.passed, "{:#?}", report.failures);
}
