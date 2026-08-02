use brain_cli::{BenchmarkProfile, benchmark_corpus, corpus_hashes};

#[test]
fn equal_seed_produces_equal_event_and_answer_hashes() {
    let first = corpus_hashes(BenchmarkProfile::Smoke, 42);
    let second = corpus_hashes(BenchmarkProfile::Smoke, 42);
    assert_eq!(first, second);
}

#[test]
fn smoke_corpus_passes_the_same_correctness_contract() {
    let temp = tempfile::tempdir().expect("temp");
    let report = benchmark_corpus(temp.path().join("smoke"), BenchmarkProfile::Smoke, 42)
        .expect("smoke benchmark");
    assert!(report.passed, "{:#?}", report.failures);
}

#[test]
#[ignore = "production release gate: generates 12,000 sessions and 6,000,000 events"]
fn primary_production_corpus_passes_all_gates() {
    let temp = tempfile::tempdir().expect("temp");
    let report = benchmark_corpus(temp.path().join("primary"), BenchmarkProfile::Primary, 42)
        .expect("primary benchmark");
    println!("{}", serde_json::to_string_pretty(&report).expect("report"));
    assert!(report.passed, "{:#?}", report.failures);
}
