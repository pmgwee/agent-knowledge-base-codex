use anyhow::{Context, Result};
use serde_json::Value;

use super::{BenchmarkHarness, NativeTrace, TraceMarkers, TraceObservation};

pub fn parse_native_trace(
    harness: BenchmarkHarness,
    raw: &str,
    markers: Option<&TraceMarkers>,
) -> Result<NativeTrace> {
    let values = parse_values(raw)?;
    let mut trace = NativeTrace::default();
    let mut observed = [None, None, None];
    for value in &values {
        scan_markers(value, markers, &mut observed);
        match harness {
            BenchmarkHarness::ClaudeCode => scan_claude(value, &mut trace),
            BenchmarkHarness::Codex => scan_codex(value, &mut trace),
        }
    }
    trace.first_correct_file = observation(observed[0]);
    trace.first_edit = observation(observed[1]);
    trace.first_passing_test = observation(observed[2]);
    Ok(trace)
}

fn parse_values(raw: &str) -> Result<Vec<Value>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(value) = serde_json::from_str(raw) {
        return Ok(vec![value]);
    }
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("native trace JSONL line {} is malformed", index + 1))
        })
        .collect()
}

fn scan_claude(value: &Value, trace: &mut NativeTrace) {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    trace.turns = trace.turns.saturating_add(1);
    let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        record_tool(trace, name, item.get("input"));
    }
}

fn scan_codex(value: &Value, trace: &mut NativeTrace) {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(event_type, "turn.started" | "turn_started") {
        trace.turns = trace.turns.saturating_add(1);
    }
    if !matches!(event_type, "item.completed" | "item_completed") {
        return;
    }
    let Some(item) = value.get("item") else {
        return;
    };
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "mcp_tool_call" => {
            let tool = item
                .get("tool")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("mcp_tool_call");
            record_tool(
                trace,
                tool,
                item.get("arguments").or_else(|| item.get("input")),
            );
        }
        "command_execution" => record_tool(trace, "command_execution", Some(item)),
        "file_change" => record_tool(trace, "file_change", Some(item)),
        _ => {}
    }
}

fn record_tool(trace: &mut NativeTrace, name: &str, input: Option<&Value>) {
    trace.total_tool_calls = trace.total_tool_calls.saturating_add(1);
    *trace.tool_calls_by_name.entry(name.to_owned()).or_insert(0) += 1;
    let lower_name = name.to_ascii_lowercase();
    if lower_name.contains("brain") {
        trace.brain_mcp_calls = trace.brain_mcp_calls.saturating_add(1);
    }
    if lower_name.contains("codegraph") {
        trace.codegraph_calls = trace.codegraph_calls.saturating_add(1);
    }

    let command = input.and_then(find_command);
    let file_read = matches!(
        lower_name.as_str(),
        "read" | "read_file" | "view_file" | "get_file"
    ) || command.is_some_and(is_file_read_command);
    if file_read {
        trace.file_read_calls = trace.file_read_calls.saturating_add(1);
    }
    if matches!(
        lower_name.as_str(),
        "edit" | "write" | "write_file" | "apply_patch" | "file_change"
    ) {
        trace.edit_calls = trace.edit_calls.saturating_add(1);
    }
    if command.is_some_and(is_test_command) {
        trace.test_commands = trace.test_commands.saturating_add(1);
    }
    if let Some(input) = input {
        collect_paths(input, &mut trace.unique_files);
    }
    if file_read {
        if let Some(command) = command {
            if let Some(path) = command_path(command) {
                trace.unique_files.insert(path);
            }
        }
    }
}

fn find_command(value: &Value) -> Option<&str> {
    value
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| value.get("cmd").and_then(Value::as_str))
}

fn is_file_read_command(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    lower.starts_with("get-content ")
        || lower.starts_with("cat ")
        || lower.starts_with("type ")
        || lower.starts_with("sed -n ")
}

fn is_test_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    [
        "cargo test",
        "pytest",
        "npm test",
        "npm run test",
        "pnpm test",
        "yarn test",
        "go test",
        "dotnet test",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn command_path(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .rev()
        .find(|part| !part.starts_with('-'))
        .map(|part| part.trim_matches(['\'', '"']).to_owned())
}

fn collect_paths(value: &Value, paths: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "file_path" | "path") {
                    if let Some(path) = value.as_str() {
                        paths.insert(path.to_owned());
                    }
                }
                collect_paths(value, paths);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn scan_markers(value: &Value, markers: Option<&TraceMarkers>, observed: &mut [Option<u64>; 3]) {
    let Some(markers) = markers else {
        return;
    };
    match value {
        Value::Object(object) => {
            if let (Some(marker), Some(elapsed_ms)) = (
                object.get("benchmark_marker").and_then(Value::as_str),
                object.get("elapsed_ms").and_then(Value::as_u64),
            ) {
                for (index, expected) in [
                    markers.first_correct_file.as_str(),
                    markers.first_edit.as_str(),
                    markers.first_passing_test.as_str(),
                ]
                .into_iter()
                .enumerate()
                {
                    if marker == expected {
                        observed[index] = Some(
                            observed[index].map_or(elapsed_ms, |current| current.min(elapsed_ms)),
                        );
                    }
                }
            }
            for value in object.values() {
                scan_markers(value, Some(markers), observed);
            }
        }
        Value::Array(values) => {
            for value in values {
                scan_markers(value, Some(markers), observed);
            }
        }
        _ => {}
    }
}

fn observation(elapsed_ms: Option<u64>) -> TraceObservation {
    elapsed_ms.map_or(TraceObservation::NotObservable, |elapsed_ms| {
        TraceObservation::Observed { elapsed_ms }
    })
}
