use std::path::{Path, PathBuf};

use anyhow::Result;
use brain_domain::{EventType, Harness, NormalizedEvent, SourceCursor};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;

use crate::jsonl::read_jsonl_increment;
use crate::{
    NormalizeContext, RawRecord, ReadOutcome, SchemaFingerprint, SourceAdapter, SourceDescriptor,
};

pub struct CodexAdapter {
    roots: Vec<PathBuf>,
}

impl CodexAdapter {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }
}

impl SourceAdapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "codex-rollout"
    }

    fn discover(&self) -> Result<Vec<SourceDescriptor>> {
        let mut sources = Vec::new();
        for root in &self.roots {
            discover_rollouts(root, &mut sources)?;
        }
        sources.sort_by(|left, right| left.path.cmp(&right.path));
        sources.dedup_by(|left, right| left.path == right.path);
        Ok(sources)
    }

    fn fingerprint(&self, source: &SourceDescriptor) -> Result<SchemaFingerprint> {
        let outcome = read_jsonl_increment(source, &SourceCursor::start())?;
        let mut shapes = Vec::new();
        if let ReadOutcome::Batch(batch) = outcome {
            for record in batch.records.iter().take(64) {
                if let Some(serde_json::Value::Object(object)) = &record.value {
                    let mut top_keys = object.keys().cloned().collect::<Vec<_>>();
                    top_keys.sort();
                    let top_type = object
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<missing>");
                    let payload = object.get("payload").and_then(serde_json::Value::as_object);
                    let payload_type = payload
                        .and_then(|value| value.get("type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<none>");
                    let role = payload
                        .and_then(|value| value.get("role"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<none>");
                    let mut payload_keys = payload
                        .map(|value| value.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    payload_keys.sort();
                    shapes.push(format!(
                        "type={top_type};payload={payload_type};role={role};top={};keys={}",
                        top_keys.join(","),
                        payload_keys.join(",")
                    ));
                }
            }
        }
        shapes.sort();
        shapes.dedup();
        let digest = Sha256::digest(shapes.join("\n").as_bytes());
        Ok(SchemaFingerprint(format!("codex-rollout:{digest:x}")))
    }

    fn read_increment(
        &self,
        source: &SourceDescriptor,
        cursor: &SourceCursor,
    ) -> Result<ReadOutcome> {
        read_jsonl_increment(source, cursor)
    }

    fn normalize(
        &self,
        record: &RawRecord,
        context: &NormalizeContext,
    ) -> Result<Vec<NormalizedEvent>> {
        let raw = record
            .value
            .clone()
            .unwrap_or_else(|| serde_json::json!({ "raw_text": record.raw_text }));
        let top_type = raw
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("malformed");
        let occurred_at = raw
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                raw.pointer("/payload/timestamp")
                    .and_then(serde_json::Value::as_str)
            })
            .and_then(|value| time::OffsetDateTime::parse(value, &Rfc3339).ok())
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let native_turn_id = raw
            .pointer("/payload/turn_id")
            .or_else(|| raw.pointer("/payload/id"))
            .or_else(|| raw.pointer("/payload/call_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let candidates = map_record(top_type, &raw, record.parse_error.as_deref());
        let native_session_id = source_session_id(record);
        let git_head = raw
            .pointer("/payload/git/commit_hash")
            .or_else(|| raw.pointer("/payload/git/commit"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let git_branch = raw
            .pointer("/payload/git/branch")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);

        candidates
            .into_iter()
            .enumerate()
            .map(|(ordinal, (event_type, payload))| {
                Ok(NormalizedEvent {
                    event_id: uuid::Uuid::now_v7(),
                    project_id: context.project_id,
                    worktree_id: context.worktree_id,
                    task_id: None,
                    harness: Harness::Codex,
                    native_session_id: native_session_id.clone(),
                    native_turn_id: native_turn_id.clone(),
                    event_type,
                    occurred_at,
                    observed_at: time::OffsetDateTime::now_utc(),
                    source_locator: record.source_locator.clone(),
                    source_offset: i64::try_from(record.byte_offset)?,
                    source_schema: context.source_schema.clone(),
                    raw_hash: record.raw_hash,
                    idempotency_key: idempotency_key(record, u32::try_from(ordinal)?),
                    git_head: git_head.clone(),
                    git_branch: git_branch.clone(),
                    payload,
                    raw: raw.clone(),
                })
            })
            .collect()
    }
}

fn map_record(
    top_type: &str,
    raw: &serde_json::Value,
    parse_error: Option<&str>,
) -> Vec<(EventType, serde_json::Value)> {
    let payload = raw
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    match top_type {
        "session_meta" => vec![(
            EventType::SessionStarted,
            select_fields(
                &payload,
                &["cwd", "cli_version", "model_provider", "source", "git"],
            ),
        )],
        "turn_context" => vec![(
            EventType::SystemObserved,
            select_fields(
                &payload,
                &[
                    "turn_id",
                    "cwd",
                    "model",
                    "summary",
                    "current_date",
                    "timezone",
                ],
            ),
        )],
        "response_item" => map_response_item(&payload),
        "event_msg" => map_event_message(&payload),
        "world_state" => vec![(EventType::CheckpointAuthored, payload)],
        "compacted" => vec![(
            EventType::SessionCompacted,
            select_fields(
                &payload,
                &[
                    "message",
                    "summary",
                    "first_window_id",
                    "previous_window_id",
                    "window_id",
                    "window_number",
                ],
            ),
        )],
        _ => vec![(
            EventType::SchemaUnknown,
            serde_json::json!({
                "native_type": top_type,
                "native_payload_type": payload.get("type"),
                "parse_error": parse_error,
            }),
        )],
    }
}

fn map_response_item(payload: &serde_json::Value) -> Vec<(EventType, serde_json::Value)> {
    let payload_type = payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<missing>");
    let event_type = match payload_type {
        "message" => match payload.get("role").and_then(serde_json::Value::as_str) {
            Some("user") => EventType::UserPrompted,
            Some("assistant") => EventType::AgentResponded,
            _ => EventType::SystemObserved,
        },
        "function_call" | "custom_tool_call" => EventType::ToolRequested,
        "function_call_output" | "custom_tool_call_output" => {
            if output_failed(payload.get("output")) {
                EventType::ToolFailed
            } else {
                EventType::ToolCompleted
            }
        }
        "reasoning" => {
            return vec![opaque_payload(payload_type, payload)];
        }
        _ => EventType::SchemaUnknown,
    };
    vec![(event_type, payload.clone())]
}

fn map_event_message(payload: &serde_json::Value) -> Vec<(EventType, serde_json::Value)> {
    let payload_type = payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<missing>");
    let event_type = match payload_type {
        "user_message" => EventType::UserPrompted,
        "agent_message" => EventType::AgentResponded,
        "agent_reasoning" => return vec![opaque_payload(payload_type, payload)],
        "context_compacted" => EventType::SessionCompacted,
        "task_complete" => EventType::TaskCompleted,
        "turn_aborted" => EventType::ToolFailed,
        "task_started" | "thread_settings_applied" | "token_count" => EventType::SystemObserved,
        _ => EventType::SchemaUnknown,
    };
    vec![(event_type, payload.clone())]
}

fn opaque_payload(
    native_payload_type: &str,
    payload: &serde_json::Value,
) -> (EventType, serde_json::Value) {
    (
        EventType::OpaqueEvidence,
        serde_json::json!({
            "native_payload_type": native_payload_type,
            "native_id": payload.get("id"),
            "has_encrypted_content": payload.get("encrypted_content").is_some(),
            "retention": "raw-only",
        }),
    )
}

fn output_failed(output: Option<&serde_json::Value>) -> bool {
    output.is_some_and(|output| {
        output.get("success").and_then(serde_json::Value::as_bool) == Some(false)
            || output.get("is_error").and_then(serde_json::Value::as_bool) == Some(true)
            || output.get("ok").and_then(serde_json::Value::as_bool) == Some(false)
    })
}

fn select_fields(value: &serde_json::Value, keys: &[&str]) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return serde_json::Value::Null;
    };
    serde_json::Value::Object(
        keys.iter()
            .filter_map(|key| {
                object
                    .get(*key)
                    .cloned()
                    .map(|value| ((*key).to_owned(), value))
            })
            .collect(),
    )
}

fn source_session_id(record: &RawRecord) -> String {
    Path::new(&record.source_locator)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown-codex-session")
        .to_owned()
}

fn idempotency_key(record: &RawRecord, ordinal: u32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"codex-rollout\0");
    hasher.update(record.source_id.as_bytes());
    hasher.update(record.byte_offset.to_le_bytes());
    hasher.update(record.raw_hash);
    hasher.update(ordinal.to_le_bytes());
    hasher.finalize().into()
}

fn discover_rollouts(root: &Path, sources: &mut Vec<SourceDescriptor>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            discover_rollouts(&path, sources)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout"))
        {
            sources.push(SourceDescriptor::file(path));
        }
    }
    Ok(())
}
