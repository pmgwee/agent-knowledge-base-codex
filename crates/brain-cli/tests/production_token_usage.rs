use brain_cli::{BenchmarkHarness, summarize_production_events};
use brain_domain::Harness;
use brain_store::NativeUsageEvent;

#[test]
fn production_usage_deduplicates_claude_messages_and_codex_cumulative_records() {
    let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(500);
    let events = vec![
        claude(now, 1, "message-1", 100),
        claude(now, 2, "message-1", 120),
        claude(now, 3, "message-2", 80),
        codex(now, 4, 400),
        codex(now, 5, 900),
    ];
    let trend = summarize_production_events(&events, now + time::Duration::hours(1), |_, _| 7);
    assert!(trend.observational);
    assert!(trend.statement.contains("does not prove token savings"));
    let month = trend
        .windows
        .iter()
        .find(|window| window.days == 30)
        .unwrap();
    let claude = month
        .harnesses
        .iter()
        .find(|row| row.harness == BenchmarkHarness::ClaudeCode)
        .unwrap();
    let codex = month
        .harnesses
        .iter()
        .find(|row| row.harness == BenchmarkHarness::Codex)
        .unwrap();
    assert_eq!(claude.total_tokens, 200);
    assert_eq!(claude.sessions, 1);
    assert_eq!(codex.total_tokens, 900);
    assert_eq!(codex.sessions, 1);
    assert_eq!(codex.brain_context_delivered, 7);
}

fn claude(now: time::OffsetDateTime, offset: i64, message: &str, total: u64) -> NativeUsageEvent {
    NativeUsageEvent {
        harness: Harness::ClaudeCode,
        native_session_id: "claude-session".to_owned(),
        occurred_at: now + time::Duration::seconds(offset),
        source_locator: "claude.jsonl".to_owned(),
        source_offset: offset,
        raw: serde_json::json!({
            "message": {"id": message},
            "usage": {
                "input_tokens": total - 10,
                "cache_creation_input_tokens": 2,
                "cache_read_input_tokens": 3,
                "output_tokens": 5
            }
        }),
    }
}

fn codex(now: time::OffsetDateTime, offset: i64, total: u64) -> NativeUsageEvent {
    NativeUsageEvent {
        harness: Harness::Codex,
        native_session_id: "codex-session".to_owned(),
        occurred_at: now + time::Duration::seconds(offset),
        source_locator: "codex.jsonl".to_owned(),
        source_offset: offset,
        raw: serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {
                "input_tokens": total - 100,
                "cached_input_tokens": 50,
                "output_tokens": 100,
                "total_tokens": total
            }}}
        }),
    }
}
