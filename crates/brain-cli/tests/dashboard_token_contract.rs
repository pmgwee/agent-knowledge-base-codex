#[test]
fn dashboard_schema_v3_carries_sessions_alerts_and_three_separate_goal_contracts() {
    assert_eq!(brain_cli::DASHBOARD_SCHEMA_VERSION, 3);
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dashboard.rs"),
    )
    .unwrap();
    assert!(source.contains("pub token_benchmark: Option<BenchmarkSummary>"));
    assert!(source.contains("pub production_tokens: ProductionTokenTrend"));
    assert!(
        source.contains(
            "pub benchmark_goals: Option<crate::token_benchmark::ThreeGoalBenchmarkReport>"
        )
    );
    assert!(source.contains("pub sessions: SessionDashboard"));
    assert!(source.contains("pub unattributed_mcp: UnattributedMcpSummary"));
    assert!(source.contains("MCP transport did not provide a native session ID"));
    assert!(source.contains("pub active_alerts: Vec<BrainAlert>"));
    assert!(source.contains("healthy_silence is not a failure"));
    assert!(source.contains("An untrusted hook is never dispatched"));
    assert!(source.contains("reply_flush_unconfirmed"));
    assert!(source.contains("without reply_flushed means delivery was not confirmed"));
}
