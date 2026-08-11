use anyhow::{Context, Result, anyhow, ensure};
use serde_json::Value;

use super::{BenchmarkHarness, NativeUsage};

pub fn parse_claude_usage(raw: &str) -> Result<NativeUsage> {
    let value: Value = serde_json::from_str(raw).context("Claude output is not valid JSON")?;
    let usage = value
        .get("usage")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Claude result has no usage object"))?;
    let input = required_u64(usage.get("input_tokens"), "input_tokens")?;
    let creation = required_u64(
        usage.get("cache_creation_input_tokens"),
        "cache_creation_input_tokens",
    )?;
    let read = required_u64(
        usage.get("cache_read_input_tokens"),
        "cache_read_input_tokens",
    )?;
    let output = required_u64(usage.get("output_tokens"), "output_tokens")?;
    let total = input
        .checked_add(creation)
        .and_then(|value| value.checked_add(read))
        .and_then(|value| value.checked_add(output))
        .ok_or_else(|| anyhow!("Claude token total overflowed"))?;
    ensure!(total > 0, "Claude token total is zero");
    Ok(NativeUsage {
        harness: BenchmarkHarness::ClaudeCode,
        input_tokens: input,
        cache_creation_input_tokens: Some(creation),
        cache_read_input_tokens: Some(read),
        cached_input_tokens: None,
        cache_write_input_tokens: None,
        output_tokens: output,
        reasoning_output_tokens: None,
        total_tokens: total,
        native_records: 1,
    })
}

pub fn parse_codex_usage(raw: &str) -> Result<NativeUsage> {
    let mut records = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .with_context(|| format!("Codex JSONL line {} is malformed", index + 1))?;
        if let Some(usage) = codex_usage_object(&value) {
            let input = required_u64(usage.get("input_tokens"), "input_tokens")?;
            let output = required_u64(usage.get("output_tokens"), "output_tokens")?;
            let total = required_u64(usage.get("total_tokens"), "total_tokens")?;
            let cached = optional_u64(usage.get("cached_input_tokens"), "cached_input_tokens")?;
            let cache_write = optional_u64(
                usage.get("cache_write_input_tokens"),
                "cache_write_input_tokens",
            )?;
            let reasoning = optional_u64(
                usage.get("reasoning_output_tokens"),
                "reasoning_output_tokens",
            )?;
            records.push((input, cached, cache_write, output, reasoning, total));
        }
    }
    ensure!(
        !records.is_empty(),
        "Codex output has no cumulative token_count record"
    );
    for window in records.windows(2) {
        ensure!(
            window[1].5 >= window[0].5,
            "Codex cumulative token total decreased"
        );
    }
    let count = u32::try_from(records.len()).context("too many Codex token records")?;
    let (input, cached, cache_write, output, reasoning, total) =
        records.pop().expect("checked non-empty");
    ensure!(total > 0, "Codex token total is zero");
    ensure!(
        total >= input.max(output),
        "Codex total is inconsistent with its components"
    );
    Ok(NativeUsage {
        harness: BenchmarkHarness::Codex,
        input_tokens: input,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
        cached_input_tokens: cached,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: reasoning,
        total_tokens: total,
        native_records: count,
    })
}

fn codex_usage_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    payload.get("info")?.get("total_token_usage")?.as_object()
}

fn required_u64(value: Option<&Value>, name: &str) -> Result<u64> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing or invalid {name}"))
}

fn optional_u64(value: Option<&Value>, name: &str) -> Result<Option<u64>> {
    value
        .map(|value| value.as_u64().ok_or_else(|| anyhow!("invalid {name}")))
        .transpose()
}
