#[test]
fn dashboard_schema_v2_carries_benchmark_and_observational_token_contracts() {
    assert_eq!(brain_cli::DASHBOARD_SCHEMA_VERSION, 2);
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dashboard.rs"),
    )
    .unwrap();
    assert!(source.contains("pub token_benchmark: Option<BenchmarkSummary>"));
    assert!(source.contains("pub production_tokens: ProductionTokenTrend"));
}
