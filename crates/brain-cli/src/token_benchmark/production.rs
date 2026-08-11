use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use brain_domain::{Harness, ProjectId};
use brain_store::{EventLedger, NativeUsageEvent};
use serde_json::Value;

use super::{BenchmarkHarness, NativeUsage, parse_claude_usage, parse_codex_usage};

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct HarnessProductionTokens {
    pub harness: BenchmarkHarness,
    pub total_tokens: u64,
    pub sessions: u64,
    pub median_tokens_per_session: f64,
    pub cached_share: f64,
    pub output_share: f64,
    pub brain_context_delivered: u64,
    pub invalid_usage_records: u64,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProductionTokenWindow {
    pub days: u32,
    pub harnesses: Vec<HarnessProductionTokens>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProductionTokenTrend {
    pub observational: bool,
    pub statement: String,
    pub windows: Vec<ProductionTokenWindow>,
}

pub fn production_token_trend(
    ledger: &EventLedger,
    project_id: ProjectId,
    now: time::OffsetDateTime,
) -> Result<ProductionTokenTrend> {
    let events = ledger.native_usage_events(project_id, now - time::Duration::days(30))?;
    let delivered = |harness: BenchmarkHarness, days: u32| {
        ledger
            .context_deliveries_by_harness(now - time::Duration::days(i64::from(days)))
            .unwrap_or_default()
            .into_iter()
            .filter(|row| match harness {
                BenchmarkHarness::ClaudeCode => row.harness == "claude-code",
                BenchmarkHarness::Codex => row.harness == "codex",
            })
            .map(|row| row.total_tokens)
            .sum()
    };
    Ok(summarize_production_events(&events, now, delivered))
}

pub fn summarize_production_events(
    events: &[NativeUsageEvent],
    now: time::OffsetDateTime,
    delivered: impl Fn(BenchmarkHarness, u32) -> u64,
) -> ProductionTokenTrend {
    let windows = [1u32, 7, 30]
        .into_iter()
        .map(|days| {
            let since = now - time::Duration::days(i64::from(days));
            let selected: Vec<_> = events
                .iter()
                .filter(|event| event.occurred_at >= since && event.occurred_at <= now)
                .collect();
            ProductionTokenWindow {
                days,
                harnesses: BenchmarkHarness::ALL
                    .into_iter()
                    .map(|harness| summarize_harness(&selected, harness, delivered(harness, days)))
                    .collect(),
            }
        })
        .collect();
    ProductionTokenTrend {
        observational: true,
        statement: "Observational production usage only; this view has no brain-off counterfactual and does not prove token savings.".to_owned(),
        windows,
    }
}

fn summarize_harness(
    events: &[&NativeUsageEvent],
    harness: BenchmarkHarness,
    brain_context_delivered: u64,
) -> HarnessProductionTokens {
    let mut invalid = 0u64;
    let usage_by_session = match harness {
        BenchmarkHarness::ClaudeCode => claude_sessions(events, &mut invalid),
        BenchmarkHarness::Codex => codex_sessions(events, &mut invalid),
    };
    let mut session_totals: Vec<u64> = usage_by_session
        .values()
        .map(|records| records.iter().map(|usage| usage.total_tokens).sum())
        .collect();
    session_totals.sort_unstable();
    let total_tokens = session_totals.iter().sum();
    let cached: u64 = usage_by_session
        .values()
        .flatten()
        .map(|usage| {
            usage.cache_creation_input_tokens.unwrap_or(0)
                + usage.cache_read_input_tokens.unwrap_or(0)
                + usage.cached_input_tokens.unwrap_or(0)
        })
        .sum();
    let output: u64 = usage_by_session
        .values()
        .flatten()
        .map(|usage| usage.output_tokens)
        .sum();
    HarnessProductionTokens {
        harness,
        total_tokens,
        sessions: session_totals.len() as u64,
        median_tokens_per_session: median(&session_totals),
        cached_share: fraction(cached, total_tokens),
        output_share: fraction(output, total_tokens),
        brain_context_delivered,
        invalid_usage_records: invalid,
    }
}

fn claude_sessions(
    events: &[&NativeUsageEvent],
    invalid: &mut u64,
) -> BTreeMap<String, Vec<NativeUsage>> {
    let mut messages: BTreeMap<(String, String), (u64, NativeUsage)> = BTreeMap::new();
    let mut invalid_messages = BTreeSet::new();
    for event in events
        .iter()
        .filter(|event| event.harness == Harness::ClaudeCode)
    {
        let Some((message_id, usage_value)) = find_claude_usage(&event.raw) else {
            *invalid += 1;
            continue;
        };
        let wrapper = serde_json::json!({ "usage": usage_value });
        let Ok(usage) = parse_claude_usage(&wrapper.to_string()) else {
            *invalid += 1;
            continue;
        };
        let key = (event.native_session_id.clone(), message_id);
        if let Some((previous, _)) = messages.get(&key)
            && usage.total_tokens < *previous
        {
            invalid_messages.insert(key.clone());
            continue;
        }
        messages.insert(key, (usage.total_tokens, usage));
    }
    for key in &invalid_messages {
        messages.remove(key);
    }
    *invalid += invalid_messages.len() as u64;
    let mut sessions: BTreeMap<String, Vec<NativeUsage>> = BTreeMap::new();
    for ((session, _message), (_total, usage)) in messages {
        sessions.entry(session).or_default().push(usage);
    }
    sessions
}

fn codex_sessions(
    events: &[&NativeUsageEvent],
    invalid: &mut u64,
) -> BTreeMap<String, Vec<NativeUsage>> {
    let mut raw_by_session: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.harness == Harness::Codex)
    {
        let mut found = Vec::new();
        find_codex_records(&event.raw, &mut found);
        if found.is_empty() {
            *invalid += 1;
        }
        raw_by_session
            .entry(event.native_session_id.clone())
            .or_default()
            .extend(found.into_iter().map(|value| value.to_string()));
    }
    let mut sessions = BTreeMap::new();
    for (session, records) in raw_by_session {
        match parse_codex_usage(&records.join("\n")) {
            Ok(usage) => {
                sessions.insert(session, vec![usage]);
            }
            Err(_) => *invalid += 1,
        }
    }
    sessions
}

fn find_claude_usage(value: &Value) -> Option<(String, Value)> {
    match value {
        Value::Object(object) => {
            if let Some(usage) = object.get("usage") {
                let message_id = object
                    .get("message")
                    .and_then(|message| message.get("id"))
                    .and_then(Value::as_str)
                    .or_else(|| object.get("id").and_then(Value::as_str));
                if let Some(message_id) = message_id {
                    return Some((message_id.to_owned(), usage.clone()));
                }
            }
            object.values().find_map(find_claude_usage)
        }
        Value::Array(values) => values.iter().find_map(find_claude_usage),
        _ => None,
    }
}

fn find_codex_records(value: &Value, records: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("event_msg")
                && object
                    .get("payload")
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str)
                    == Some("token_count")
            {
                records.push(value.clone());
                return;
            }
            for value in object.values() {
                find_codex_records(value, records);
            }
        }
        Value::Array(values) => {
            for value in values {
                find_codex_records(value, records);
            }
        }
        _ => {}
    }
}

fn median(sorted: &[u64]) -> f64 {
    match sorted.len() {
        0 => 0.0,
        length if length % 2 == 1 => sorted[length / 2] as f64,
        length => (sorted[length / 2 - 1] as f64 + sorted[length / 2] as f64) / 2.0,
    }
}

fn fraction(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
