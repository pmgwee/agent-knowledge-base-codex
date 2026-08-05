#![forbid(unsafe_code)]

mod spool;

pub mod protocol;

use std::path::PathBuf;
use std::time::Duration;

use brain_domain::{HOOK_PROTOCOL_VERSION, Harness, HookEnvelope, HookReply};

pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\agent-brain-v1";

/// The ceiling the hook waits for the service to compile and return an orientation.
///
/// This blocks session start, so the number is a real cost — but only when the service is
/// slow or absent. A healthy service compiles in ~130 ms and replies through the pipe in
/// ~150–300 ms, so a normal session never approaches this bound; it only bites when the
/// service is unreachable, in which case the hook fails open to `{}` after this long.
///
/// 250 ms was correct when written and became wrong silently: the service serves the pipe
/// on a reactor it shares with consolidation/projections/rediscovery, and blocking SQLite
/// in those loops could starve the accept loop for ~1 s, so every hook hit the ceiling.
/// The service now runs the pipe on its own task with the handler in `spawn_blocking`, so
/// the pipe path is back to compile-bound (~150 ms). 3 s is ~10× that headroom, covering
/// cold caches, disk contention, and a burst of concurrent session starts — while still
/// bounding the damage when the service is genuinely down. Re-measure before lowering.
pub const HOOK_HARD_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct HookOptions {
    pub brain_home: PathBuf,
    pub pipe_name: String,
    pub harness: Harness,
    pub event_name: Option<String>,
    pub timeout: Duration,
}

pub async fn invoke(options: HookOptions, input: &[u8]) -> serde_json::Value {
    let mut payload = parse_payload(input);
    if let Some(object) = payload.as_object_mut() {
        if !object.contains_key("brain_task_id")
            && let Some(task_id) = std::env::var_os("BRAIN_TASK_ID")
        {
            object.insert(
                "brain_task_id".to_owned(),
                serde_json::Value::String(task_id.to_string_lossy().to_string()),
            );
        }
        if !object.contains_key("session_id")
            && !object.contains_key("sessionId")
            && let Some(session_id) = std::env::var_os("BRAIN_NATIVE_SESSION_ID")
        {
            object.insert(
                "session_id".to_owned(),
                serde_json::Value::String(session_id.to_string_lossy().to_string()),
            );
        }
    }
    let event_name = options
        .event_name
        .clone()
        .or_else(|| {
            payload
                .get("hook_event_name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            payload
                .get("hookEventName")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unknown".to_owned());
    let envelope = HookEnvelope {
        protocol: HOOK_PROTOCOL_VERSION,
        harness: options.harness.clone(),
        event_name: event_name.clone(),
        received_at: time::OffsetDateTime::now_utc(),
        nonce: uuid::Uuid::now_v7(),
        payload,
    };

    match protocol::request(&options.pipe_name, &envelope, options.timeout).await {
        Ok(reply) => render_reply(&options.harness, &event_name, reply),
        Err(error) => {
            if let Err(spool_error) = spool::write(&options.brain_home, &envelope) {
                eprintln!("brain-hook spool failure: {spool_error:#}");
            }
            eprintln!("brain-hook service unavailable: {error:#}");
            serde_json::json!({})
        }
    }
}

fn parse_payload(input: &[u8]) -> serde_json::Value {
    serde_json::from_slice(input).unwrap_or_else(|error| {
        serde_json::json!({
            "unparsed": String::from_utf8_lossy(input),
            "parse_error": error.to_string(),
        })
    })
}

fn render_reply(harness: &Harness, event_name: &str, reply: HookReply) -> serde_json::Value {
    let Some(context) = reply.additional_context else {
        return serde_json::json!({});
    };
    match harness {
        Harness::ClaudeCode | Harness::Codex => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": event_name,
                "additionalContext": context,
            }
        }),
        Harness::Hermes | Harness::Other(_) => {
            serde_json::json!({ "additional_context": context })
        }
    }
}

#[cfg(test)]
mod tests {
    use brain_domain::{Harness, HookReply};

    use super::render_reply;

    #[test]
    fn claude_session_start_reply_uses_the_native_hook_shape() {
        let output = render_reply(
            &Harness::ClaudeCode,
            "SessionStart",
            HookReply {
                additional_context: Some("bounded context".to_owned()),
                diagnostics_id: Some("diagnostic".to_owned()),
            },
        );

        assert_eq!(
            output,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": "bounded context",
                }
            })
        );
    }

    #[test]
    fn capture_only_reply_is_an_empty_object() {
        assert_eq!(
            render_reply(&Harness::ClaudeCode, "PostToolUse", HookReply::default()),
            serde_json::json!({})
        );
    }

    #[test]
    fn codex_session_start_reply_uses_the_native_lifecycle_hook_shape() {
        let output = render_reply(
            &Harness::Codex,
            "SessionStart",
            HookReply {
                additional_context: Some("bounded context".to_owned()),
                diagnostics_id: None,
            },
        );

        assert_eq!(
            output,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": "bounded context",
                }
            })
        );
    }
}
