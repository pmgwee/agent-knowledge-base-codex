use brain_cli::{BenchmarkProfile, benchmark_corpus};

#[test]
#[ignore = "overnight release-candidate gate: 120,000 sessions and 60,000,000 events"]
fn stress_corpus_keeps_startup_query_and_memory_bounds() {
    let temp = tempfile::tempdir().expect("temp");
    let report = benchmark_corpus(temp.path().join("stress"), BenchmarkProfile::Stress, 42)
        .expect("stress benchmark");
    println!("{}", serde_json::to_string_pretty(&report).expect("report"));
    assert!(report.passed, "{:#?}", report.failures);
}
