use brain_cli::{BenchmarkHarness, NativeUsage, parse_claude_usage, parse_codex_usage};

#[test]
fn claude_total_includes_both_cache_classes() {
    let raw = include_str!("fixtures/token-benchmark/claude-result.json");
    let usage = parse_claude_usage(raw).expect("valid Claude usage");
    assert_eq!(usage.harness, BenchmarkHarness::ClaudeCode);
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.cache_creation_input_tokens, Some(20));
    assert_eq!(usage.cache_read_input_tokens, Some(300));
    assert_eq!(usage.output_tokens, 40);
    assert_eq!(usage.total_tokens, 460);
}

#[test]
fn claude_missing_cache_counter_is_invalid_instead_of_estimated() {
    let raw = include_str!("fixtures/token-benchmark/claude-missing-cache.json");
    assert!(
        parse_claude_usage(raw)
            .unwrap_err()
            .to_string()
            .contains("cache_read_input_tokens")
    );
}

#[test]
fn codex_uses_final_cumulative_total_without_double_counting_breakdowns() {
    let raw = include_str!("fixtures/token-benchmark/codex-events.jsonl");
    let usage = parse_codex_usage(raw).expect("valid Codex usage");
    assert_eq!(usage.harness, BenchmarkHarness::Codex);
    assert_eq!(usage.input_tokens, 700);
    assert_eq!(usage.cached_input_tokens, Some(500));
    assert_eq!(usage.cache_write_input_tokens, Some(25));
    assert_eq!(usage.output_tokens, 200);
    assert_eq!(usage.reasoning_output_tokens, Some(80));
    assert_eq!(usage.total_tokens, 900);
}

#[test]
fn codex_rejects_a_decreasing_cumulative_stream() {
    let raw = include_str!("fixtures/token-benchmark/codex-decreasing.jsonl");
    assert!(
        parse_codex_usage(raw)
            .unwrap_err()
            .to_string()
            .contains("decreased")
    );
}

#[test]
fn native_usage_round_trips_without_losing_components() {
    let usage = NativeUsage {
        harness: BenchmarkHarness::Codex,
        input_tokens: 10,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
        cached_input_tokens: Some(5),
        cache_write_input_tokens: Some(1),
        output_tokens: 4,
        reasoning_output_tokens: Some(2),
        total_tokens: 14,
        native_records: 3,
    };
    let encoded = serde_json::to_string(&usage).unwrap();
    assert_eq!(
        serde_json::from_str::<NativeUsage>(&encoded).unwrap(),
        usage
    );
}
