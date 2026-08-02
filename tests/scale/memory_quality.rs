use brain_cli::{BenchmarkProfile, benchmark_corpus};

#[test]
#[ignore = "quality release gate runs over the full primary corpus"]
fn evidence_supported_precision_recall_and_isolation_meet_the_locked_gate() {
    let temp = tempfile::tempdir().expect("temp");
    let report = benchmark_corpus(temp.path().join("quality"), BenchmarkProfile::Primary, 7)
        .expect("quality benchmark");
    assert!(report.historical_precision >= 0.95);
    assert!(report.historical_recall >= 0.95);
    assert_eq!(report.project_leakage_hits, 0);
    assert!(report.supersession_fixtures_correct);
}
