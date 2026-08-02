use std::path::{Path, PathBuf};

use anyhow::Result;
use brain_domain::{EventType, Harness, NormalizedEvent, SourceCursor};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;

use crate::jsonl::read_jsonl_increment;
use crate::traits::{
    NormalizeContext, RawRecord, ReadOutcome, SchemaFingerprint, SourceAdapter, SourceDescriptor,
};

pub struct ClaudeAdapter {
    projects_root: PathBuf,
}

impl ClaudeAdapter {
    pub fn new(projects_root: impl Into<PathBuf>) -> Self {
        Self {
            projects_root: projects_root.into(),
        }
    }
}

impl SourceAdapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude-jsonl"
    }

    fn discover(&self) -> Result<Vec<SourceDescriptor>> {
        let mut sources = Vec::new();
        discover_jsonl(&self.projects_root, &mut sources)?;
        sources.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(sources)
    }

    fn fingerprint(&self, source: &SourceDescriptor) -> Result<SchemaFingerprint> {
        let outcome = read_jsonl_increment(source, &SourceCursor::start())?;
        let mut shapes = Vec::new();
        if let ReadOutcome::Batch(batch) = outcome {
            for record in batch.records.iter().take(32) {
                if let Some(serde_json::Value::Object(object)) = &record.value {
                    let mut keys = object.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    let native_type = object
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<missing>");
                    shapes.push(format!("type={native_type};keys={}", keys.join(",")));
                }
            }
        }
        shapes.sort();
        shapes.dedup();
        let digest = Sha256::digest(shapes.join("\n").as_bytes());
        Ok(SchemaFingerprint(format!("claude-jsonl:{digest:x}")))
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
        let native_type = raw
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("malformed");
        let native_session_id = raw
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown-session")
            .to_owned();
        let native_turn_id = raw
            .get("uuid")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let occurred_at = raw
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| time::OffsetDateTime::parse(value, &Rfc3339).ok())
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let mut candidates = Vec::new();
        match native_type {
            "user" => {
                let tool_results = message_blocks(&raw)
                    .filter(|block| {
                        block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let has_prompt_content =
                    raw.pointer("/message/content")
                        .is_some_and(|content| match content {
                            serde_json::Value::String(text) => !text.is_empty(),
                            serde_json::Value::Array(blocks) => blocks.iter().any(|block| {
                                matches!(
                                    block.get("type").and_then(serde_json::Value::as_str),
                                    Some("text" | "image" | "document")
                                )
                            }),
                            _ => false,
                        });
                if has_prompt_content || tool_results.is_empty() {
                    candidates.push((
                        EventType::UserPrompted,
                        serde_json::json!({ "message": raw.get("message") }),
                    ));
                }
                for result in tool_results {
                    let event_type = if result
                        .get("is_error")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        EventType::ToolFailed
                    } else {
                        EventType::ToolCompleted
                    };
                    candidates.push((event_type, result));
                }
            }
            "assistant" => {
                candidates.push((
                    EventType::AgentResponded,
                    serde_json::json!({
                        "message": raw.get("message"),
                        "request_id": raw.get("requestId"),
                    }),
                ));
                for block in message_blocks(&raw).filter(|block| {
                    matches!(
                        block.get("type").and_then(serde_json::Value::as_str),
                        Some("tool_use" | "server_tool_use")
                    )
                }) {
                    candidates.push((EventType::ToolRequested, block.clone()));
                }
            }
            "system" => candidates.push((
                EventType::SystemObserved,
                serde_json::json!({
                    "subtype": raw.get("subtype"),
                    "level": raw.get("level"),
                    "stop_reason": raw.get("stopReason"),
                }),
            )),
            "attachment" => candidates.push((
                EventType::AttachmentObserved,
                raw.get("attachment")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            )),
            "queue-operation" => candidates.push((
                EventType::QueueOperationObserved,
                serde_json::json!({
                    "operation": raw.get("operation"),
                    "content": raw.get("content"),
                }),
            )),
            "mode" => candidates.push((
                EventType::ModeChanged,
                serde_json::json!({ "mode": raw.get("mode") }),
            )),
            "relocated" => candidates.push((
                EventType::SessionRelocated,
                serde_json::json!({
                    "from": raw.get("from"),
                    "to": raw.get("to"),
                }),
            )),
            _ => candidates.push((
                EventType::SchemaUnknown,
                serde_json::json!({
                    "native_type": native_type,
                    "parse_error": record.parse_error,
                }),
            )),
        }

        candidates
            .into_iter()
            .enumerate()
            .map(|(ordinal, (event_type, payload))| {
                Ok(NormalizedEvent {
                    event_id: uuid::Uuid::now_v7(),
                    project_id: context.project_id,
                    worktree_id: context.worktree_id,
                    task_id: None,
                    harness: Harness::ClaudeCode,
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
                    git_head: None,
                    git_branch: raw
                        .get("gitBranch")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    payload,
                    raw: raw.clone(),
                })
            })
            .collect()
    }
}

fn discover_jsonl(root: &Path, sources: &mut Vec<SourceDescriptor>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            discover_jsonl(&path, sources)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            sources.push(SourceDescriptor::file(path));
        }
    }
    Ok(())
}

fn idempotency_key(record: &RawRecord, ordinal: u32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"claude-jsonl\0");
    hasher.update(record.source_id.as_bytes());
    hasher.update(record.byte_offset.to_le_bytes());
    hasher.update(record.raw_hash);
    hasher.update(ordinal.to_le_bytes());
    hasher.finalize().into()
}

fn message_blocks(raw: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    raw.pointer("/message/content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
}
