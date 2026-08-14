use brain_cli::{
    BenchmarkHarness, TraceMarkers, TraceObservation, parse_codex_usage, parse_native_trace,
};

#[test]
fn claude_trace_counts_tools_files_and_explicit_milestones() {
    let raw = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/lib.rs\"}}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"mcp__brain__brain_search\",\"input\":{}}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"mcp__codegraph__codegraph_explore\",\"input\":{}}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"src/lib.rs\"}}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo test -p fixture\"}}]}}\n",
        "{\"benchmark_marker\":\"correct-file\",\"elapsed_ms\":120,\"timestamp\":\"2099-01-01T00:00:00Z\"}\n",
        "{\"benchmark_marker\":\"first-edit\",\"elapsed_ms\":230}\n",
        "{\"benchmark_marker\":\"passing-test\",\"elapsed_ms\":450}\n"
    );
    let markers = TraceMarkers {
        first_correct_file: "correct-file".to_owned(),
        first_edit: "first-edit".to_owned(),
        first_passing_test: "passing-test".to_owned(),
    };
    let trace = parse_native_trace(BenchmarkHarness::ClaudeCode, raw, Some(&markers)).unwrap();
    assert_eq!(trace.turns, 5);
    assert_eq!(trace.total_tool_calls, 5);
    assert_eq!(trace.tool_calls_by_name["Read"], 1);
    assert_eq!(trace.brain_mcp_calls, 1);
    assert_eq!(trace.codegraph_calls, 1);
    assert_eq!(trace.file_read_calls, 1);
    assert_eq!(
        trace.unique_files,
        ["src/lib.rs".to_owned()].into_iter().collect()
    );
    assert_eq!(trace.edit_calls, 1);
    assert_eq!(trace.test_commands, 1);
    assert_eq!(
        trace.first_correct_file,
        TraceObservation::Observed { elapsed_ms: 120 }
    );
    assert_eq!(
        trace.first_edit,
        TraceObservation::Observed { elapsed_ms: 230 }
    );
    assert_eq!(
        trace.first_passing_test,
        TraceObservation::Observed { elapsed_ms: 450 }
    );
}

#[test]
fn codex_trace_uses_completed_items_and_never_sums_cumulative_usage() {
    let raw = concat!(
        "{\"type\":\"turn.started\",\"timestamp\":\"2020-01-01T00:00:00Z\"}\n",
        "{\"type\":\"item.started\",\"item\":{\"id\":\"1\",\"type\":\"mcp_tool_call\",\"server\":\"codegraph\",\"tool\":\"codegraph_explore\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"1\",\"type\":\"mcp_tool_call\",\"server\":\"codegraph\",\"tool\":\"codegraph_explore\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"2\",\"type\":\"mcp_tool_call\",\"server\":\"brain\",\"tool\":\"brain_search\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"3\",\"type\":\"command_execution\",\"command\":\"Get-Content src/main.rs\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"4\",\"type\":\"file_change\",\"path\":\"src/main.rs\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"5\",\"type\":\"command_execution\",\"command\":\"cargo test --workspace\"}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":300,\"output_tokens\":100,\"total_tokens\":400}}}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":700,\"output_tokens\":200,\"total_tokens\":900}}}}\n"
    );
    let trace = parse_native_trace(BenchmarkHarness::Codex, raw, None).unwrap();
    assert_eq!(trace.turns, 1);
    assert_eq!(trace.total_tool_calls, 5);
    assert_eq!(trace.brain_mcp_calls, 1);
    assert_eq!(trace.codegraph_calls, 1);
    assert_eq!(trace.file_read_calls, 1);
    assert_eq!(trace.edit_calls, 1);
    assert_eq!(trace.test_commands, 1);
    assert_eq!(trace.first_correct_file, TraceObservation::NotObservable);
    assert_eq!(trace.first_edit, TraceObservation::NotObservable);
    assert_eq!(trace.first_passing_test, TraceObservation::NotObservable);

    let usage = parse_codex_usage(raw).unwrap();
    assert_eq!(usage.total_tokens, 900);
    assert_eq!(usage.native_records, 2);
}

#[test]
fn wall_clock_timestamps_are_not_reinterpreted_as_elapsed_milestones() {
    let raw = concat!(
        "{\"type\":\"turn.started\",\"timestamp\":\"2000-01-01T00:00:00Z\"}\n",
        "{\"type\":\"item.completed\",\"timestamp\":\"2100-01-01T00:00:00Z\",\"item\":{\"type\":\"file_change\",\"path\":\"src/lib.rs\"}}\n"
    );
    let markers = TraceMarkers {
        first_correct_file: "correct-file".to_owned(),
        first_edit: "first-edit".to_owned(),
        first_passing_test: "passing-test".to_owned(),
    };
    let trace = parse_native_trace(BenchmarkHarness::Codex, raw, Some(&markers)).unwrap();
    assert_eq!(trace.first_correct_file, TraceObservation::NotObservable);
    assert_eq!(trace.first_edit, TraceObservation::NotObservable);
    assert_eq!(trace.first_passing_test, TraceObservation::NotObservable);
}
