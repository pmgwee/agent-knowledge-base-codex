use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::{EventType, Harness, NormalizedEvent, SourceCursor};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use crate::jsonl::file_identity;
use crate::{
    FileRotation, NormalizeContext, RawRecord, RawRecordBatch, ReadOutcome, SchemaDrift,
    SchemaFingerprint, SourceAdapter, SourceDescriptor,
};

const REVIEWED_SCHEMA: &str = include_str!("../../../fixtures/hermes/schema.sql");
const MAX_ROWS_PER_BATCH: i64 = 1_000;

#[derive(Clone, Debug)]
pub struct HermesSchemaProfile {
    pub fingerprint: SchemaFingerprint,
    pub session_table: String,
    pub message_table: String,
    pub message_order_columns: Vec<String>,
}

impl HermesSchemaProfile {
    pub fn reviewed_v22() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(REVIEWED_SCHEMA)?;
        Ok(Self {
            fingerprint: fingerprint_connection(&connection)?,
            session_table: "sessions".to_owned(),
            message_table: "messages".to_owned(),
            message_order_columns: vec!["session_id".to_owned(), "id".to_owned()],
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HermesActivation {
    Active {
        fingerprint: SchemaFingerprint,
    },
    FixtureOnly {
        expected: SchemaFingerprint,
        observed: SchemaFingerprint,
        reason: String,
    },
}

pub struct HermesAdapter {
    database_path: PathBuf,
    profile: HermesSchemaProfile,
    project_root: Option<PathBuf>,
}

impl HermesAdapter {
    /// Fixture/review mode can inspect a synthetic database but cannot report
    /// production activation because no project boundary is bound.
    pub fn reviewed(database_path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            database_path: database_path.into(),
            profile: HermesSchemaProfile::reviewed_v22()?,
            project_root: None,
        })
    }

    pub fn reviewed_for_project(
        database_path: impl Into<PathBuf>,
        project_root: impl AsRef<Path>,
    ) -> Result<Self> {
        let project_root = std::fs::canonicalize(project_root.as_ref()).with_context(|| {
            format!(
                "resolve Hermes project root {}",
                project_root.as_ref().display()
            )
        })?;
        Ok(Self {
            database_path: database_path.into(),
            profile: HermesSchemaProfile::reviewed_v22()?,
            project_root: Some(project_root),
        })
    }

    pub fn activation(&self) -> Result<HermesActivation> {
        let observed = self.fingerprint(&SourceDescriptor::file(&self.database_path))?;
        if observed != self.profile.fingerprint {
            return Ok(HermesActivation::FixtureOnly {
                expected: self.profile.fingerprint.clone(),
                observed,
                reason: "installed Hermes schema does not match reviewed v22 profile".to_owned(),
            });
        }
        if self.project_root.is_none() {
            return Ok(HermesActivation::FixtureOnly {
                expected: self.profile.fingerprint.clone(),
                observed,
                reason: "no canonical project root is bound".to_owned(),
            });
        }
        Ok(HermesActivation::Active {
            fingerprint: observed,
        })
    }

    fn ensure_source(&self, source: &SourceDescriptor) -> Result<()> {
        let expected = std::fs::canonicalize(&self.database_path).with_context(|| {
            format!(
                "resolve configured Hermes database {}",
                self.database_path.display()
            )
        })?;
        let observed = std::fs::canonicalize(&source.path)
            .with_context(|| format!("resolve Hermes source {}", source.path.display()))?;
        ensure!(
            normalize_path(&expected) == normalize_path(&observed),
            "Hermes source {} is not the configured database {}",
            source.path.display(),
            self.database_path.display()
        );
        Ok(())
    }
}

impl SourceAdapter for HermesAdapter {
    fn name(&self) -> &'static str {
        "hermes-state"
    }

    fn discover(&self) -> Result<Vec<SourceDescriptor>> {
        Ok(self
            .database_path
            .is_file()
            .then(|| SourceDescriptor::file(&self.database_path))
            .into_iter()
            .collect())
    }

    fn fingerprint(&self, source: &SourceDescriptor) -> Result<SchemaFingerprint> {
        self.ensure_source(source)?;
        fingerprint_connection(&open_read_only(&source.path)?)
    }

    fn read_increment(
        &self,
        source: &SourceDescriptor,
        cursor: &SourceCursor,
    ) -> Result<ReadOutcome> {
        self.ensure_source(source)?;
        let observed = self.fingerprint(source)?;
        if observed != self.profile.fingerprint {
            let sample_hash = Sha256::digest(observed.0.as_bytes()).into();
            return Ok(ReadOutcome::SchemaDrift(SchemaDrift {
                source_id: source.source_id.clone(),
                expected: self.profile.fingerprint.clone(),
                observed,
                sample_hash,
            }));
        }

        let identity = file_identity(&source.path)?;
        let rotated = cursor
            .file_identity
            .as_ref()
            .is_some_and(|previous| previous != &identity.0);
        let start_message_id = if rotated {
            0
        } else {
            cursor_message_id(cursor)?
        };
        let rotation = rotated.then(|| FileRotation {
            previous_identity: cursor.file_identity.clone(),
            current_identity: identity.0.clone(),
            previous_offset: cursor.byte_offset,
            current_size: std::fs::metadata(&source.path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        });
        let connection = open_read_only(&source.path)?;
        let rows = read_rows(&connection, start_message_id, self.project_root.as_deref())?;
        if rows.is_empty() && rotation.is_none() {
            return Ok(ReadOutcome::NoChange);
        }
        let last_message_id = rows
            .last()
            .and_then(|record| record.value.as_ref())
            .and_then(|value| value.pointer("/message/id"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(start_message_id);
        let last_session_id = rows
            .last()
            .and_then(|record| record.value.as_ref())
            .and_then(|value| value.pointer("/session/id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let committed = u64::try_from(last_message_id)?;
        Ok(ReadOutcome::Batch(RawRecordBatch {
            records: rows,
            next_cursor: SourceCursor::for_native(
                committed,
                identity.0.clone(),
                serde_json::json!({
                    "session_id": last_session_id,
                    "message_id": last_message_id,
                }),
            ),
            file_identity: identity,
            last_complete_newline: committed,
            rotation,
        }))
    }

    fn normalize(
        &self,
        record: &RawRecord,
        context: &NormalizeContext,
    ) -> Result<Vec<NormalizedEvent>> {
        let raw = record
            .value
            .clone()
            .context("Hermes row has no reviewed JSON value")?;
        let message = raw.get("message").context("Hermes row has no message")?;
        let session = raw.get("session").context("Hermes row has no session")?;
        let role = message
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let mut candidates = Vec::new();
        if raw
            .get("is_first_message")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            candidates.push((
                EventType::SessionStarted,
                select_fields(
                    session,
                    &[
                        "id",
                        "source",
                        "model",
                        "cwd",
                        "git_branch",
                        "git_repo_root",
                        "title",
                        "parent_session_id",
                        "started_at",
                    ],
                ),
            ));
        }
        let message_type = match role {
            "user" => EventType::UserPrompted,
            "assistant" => EventType::AgentResponded,
            "tool" => {
                if tool_failed(message) {
                    EventType::ToolFailed
                } else {
                    EventType::ToolCompleted
                }
            }
            "system" => EventType::SystemObserved,
            _ => EventType::SchemaUnknown,
        };
        candidates.push((
            message_type,
            select_fields(
                message,
                &[
                    "id",
                    "role",
                    "content",
                    "tool_call_id",
                    "tool_calls",
                    "tool_name",
                    "effect_disposition",
                    "finish_reason",
                    "active",
                    "compacted",
                ],
            ),
        ));
        if has_reasoning(message) {
            candidates.push((
                EventType::OpaqueEvidence,
                serde_json::json!({
                    "native_payload_type": "hermes-message-reasoning",
                    "message_id": message.get("id"),
                    "retention": "raw-only",
                }),
            ));
        }

        let native_session_id = session
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown-hermes-session")
            .to_owned();
        let native_turn_id = message
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string());
        let occurred_at = message
            .get("timestamp")
            .and_then(serde_json::Value::as_f64)
            .and_then(epoch_seconds)
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let git_branch = session
            .get("git_branch")
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
                    harness: Harness::Hermes,
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
                    git_branch: git_branch.clone(),
                    payload,
                    raw: raw.clone(),
                })
            })
            .collect()
    }
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.busy_timeout(std::time::Duration::from_millis(250))?;
    connection.execute_batch("PRAGMA query_only=ON;")?;
    Ok(connection)
}

fn fingerprint_connection(connection: &Connection) -> Result<SchemaFingerprint> {
    let version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .context("Hermes schema_version is missing or unreadable")?;
    let mut signature = format!("version={version}\n");
    for table in ["sessions", "messages"] {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement.query_map([], |row| {
            Ok(format!(
                "{}|{}|{}|{}|{}|{}",
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                row.get::<_, i64>(5)?,
            ))
        })?;
        let columns = columns.collect::<Result<Vec<_>, _>>()?;
        ensure!(!columns.is_empty(), "Hermes table {table} is missing");
        signature.push_str(table);
        signature.push('\n');
        signature.push_str(&columns.join("\n"));
        signature.push('\n');
    }
    let digest = Sha256::digest(signature.as_bytes());
    Ok(SchemaFingerprint(format!("hermes-state:{digest:x}")))
}

fn cursor_message_id(cursor: &SourceCursor) -> Result<i64> {
    if let Some(native) = cursor.native_position.as_ref() {
        let message_id = native
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .context("Hermes cursor has no integer message_id")?;
        ensure!(
            u64::try_from(message_id)? == cursor.byte_offset,
            "Hermes composite cursor disagrees with its monotonic offset"
        );
        return Ok(message_id);
    }
    Ok(i64::try_from(cursor.byte_offset)?)
}

fn read_rows(
    connection: &Connection,
    after_message_id: i64,
    project_root: Option<&Path>,
) -> Result<Vec<RawRecord>> {
    let effective_cwd = "COALESCE(NULLIF(TRIM(s.cwd), ''), (SELECT p.cwd FROM sessions p WHERE p.id = s.parent_session_id))";
    let (query, arguments) = if let Some(root) = project_root {
        let root = without_windows_verbatim_prefix(root.to_string_lossy().as_ref());
        (
            format!(
                "{} WHERE m.id > ?1 AND ({effective_cwd} = ?2 OR {effective_cwd} LIKE ?3 OR {effective_cwd} LIKE ?4) ORDER BY m.id LIMIT ?5",
                row_select()
            ),
            vec![
                rusqlite::types::Value::Integer(after_message_id),
                rusqlite::types::Value::Text(root.clone()),
                rusqlite::types::Value::Text(format!("{root}\\%")),
                rusqlite::types::Value::Text(format!("{root}/%")),
                rusqlite::types::Value::Integer(MAX_ROWS_PER_BATCH),
            ],
        )
    } else {
        (
            format!("{} WHERE m.id > ?1 ORDER BY m.id LIMIT ?2", row_select()),
            vec![
                rusqlite::types::Value::Integer(after_message_id),
                rusqlite::types::Value::Integer(MAX_ROWS_PER_BATCH),
            ],
        )
    };
    let mut statement = connection.prepare(&query)?;
    let mapped = statement.query_map(rusqlite::params_from_iter(arguments), |row| {
        let message_id: i64 = row.get(0)?;
        let session_id: String = row.get(1)?;
        let mut message = serde_json::Map::new();
        for (index, name) in MESSAGE_COLUMNS.iter().enumerate() {
            message.insert((*name).to_owned(), sqlite_json(row.get_ref(index)?));
        }
        let mut session = serde_json::Map::new();
        for (offset, name) in SESSION_COLUMNS.iter().enumerate() {
            session.insert(
                (*name).to_owned(),
                sqlite_json(row.get_ref(MESSAGE_COLUMNS.len() + offset)?),
            );
        }
        let is_first_message: i64 = row.get(MESSAGE_COLUMNS.len() + SESSION_COLUMNS.len())?;
        let value = serde_json::json!({
            "message": message,
            "session": session,
            "is_first_message": is_first_message != 0,
        });
        let raw_text = value.to_string();
        let raw_hash = Sha256::digest(raw_text.as_bytes()).into();
        Ok((message_id, session_id, value, raw_text, raw_hash))
    })?;
    let source_path: String = connection
        .path()
        .map(str::to_owned)
        .unwrap_or_else(|| "hermes-state.db".to_owned());
    let source = SourceDescriptor::file(&source_path);
    let mut records = Vec::new();
    for row in mapped {
        let (message_id, _session_id, value, raw_text, raw_hash) = row?;
        let next = u64::try_from(message_id)?;
        records.push(RawRecord {
            source_id: source.source_id.clone(),
            source_locator: source_path.clone(),
            byte_offset: next.saturating_sub(1),
            next_byte_offset: next,
            value: Some(value),
            raw_text,
            parse_error: None,
            raw_hash,
        });
    }
    Ok(records)
}

const MESSAGE_COLUMNS: &[&str] = &[
    "id",
    "session_id",
    "role",
    "content",
    "tool_call_id",
    "tool_calls",
    "tool_name",
    "effect_disposition",
    "timestamp",
    "token_count",
    "finish_reason",
    "reasoning",
    "reasoning_content",
    "reasoning_details",
    "codex_reasoning_items",
    "codex_message_items",
    "platform_message_id",
    "observed",
    "active",
    "compacted",
    "api_content",
];

const SESSION_COLUMNS: &[&str] = &[
    "id",
    "source",
    "model",
    "cwd",
    "git_branch",
    "git_repo_root",
    "title",
    "parent_session_id",
    "started_at",
    "ended_at",
    "end_reason",
];

fn row_select() -> String {
    let messages = MESSAGE_COLUMNS
        .iter()
        .map(|column| format!("m.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sessions = SESSION_COLUMNS
        .iter()
        .map(|column| format!("s.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT {messages}, {sessions}, m.id = (SELECT MIN(first.id) FROM messages first WHERE first.session_id = m.session_id) FROM messages m JOIN sessions s ON s.id = m.session_id"
    )
}

fn sqlite_json(value: ValueRef<'_>) -> serde_json::Value {
    match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(value) => serde_json::json!(value),
        ValueRef::Real(value) => serde_json::json!(value),
        ValueRef::Text(value) => serde_json::Value::String(String::from_utf8_lossy(value).into()),
        ValueRef::Blob(value) => serde_json::json!({"blob_hex": hex::encode(value)}),
    }
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

fn has_reasoning(message: &serde_json::Value) -> bool {
    [
        "reasoning",
        "reasoning_content",
        "reasoning_details",
        "codex_reasoning_items",
    ]
    .into_iter()
    .any(|key| !message.get(key).is_none_or(serde_json::Value::is_null))
}

fn tool_failed(message: &serde_json::Value) -> bool {
    message
        .get("effect_disposition")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| matches!(value.to_lowercase().as_str(), "error" | "failed" | "denied"))
}

fn epoch_seconds(value: f64) -> Option<time::OffsetDateTime> {
    if !value.is_finite() {
        return None;
    }
    time::OffsetDateTime::from_unix_timestamp_nanos((value * 1_000_000_000.0) as i128).ok()
}

fn idempotency_key(record: &RawRecord, ordinal: u32) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"hermes-state\0");
    hasher.update(record.source_id.as_bytes());
    hasher.update(record.byte_offset.to_le_bytes());
    hasher.update(record.raw_hash);
    hasher.update(ordinal.to_le_bytes());
    hasher.finalize().into()
}

fn normalize_path(path: &Path) -> String {
    without_windows_verbatim_prefix(path.to_string_lossy().as_ref())
        .replace('/', "\\")
        .to_lowercase()
}

fn without_windows_verbatim_prefix(path: &str) -> String {
    path.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| path.to_owned())
}
