use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use brain_domain::{Harness, ProjectId};
use rusqlite::{OptionalExtension, params};

use crate::EventLedger;
use crate::cursor::timestamp_ns;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleChannel {
    SessionStart,
    UserPromptSubmit,
    SessionEnd,
    BrainMcp,
    Capture,
}

impl LifecycleChannel {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::SessionEnd => "session_end",
            Self::BrainMcp => "brain_mcp",
            Self::Capture => "capture",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "session_start" => Self::SessionStart,
            "user_prompt_submit" => Self::UserPromptSubmit,
            "session_end" => Self::SessionEnd,
            "brain_mcp" => Self::BrainMcp,
            "capture" => Self::Capture,
            _ => bail!("unknown lifecycle channel {value:?}"),
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    HookReceived,
    RetrievalDecided,
    ReplyFlushed,
    McpRequest,
    McpSucceeded,
    McpFailed,
    SessionEndPersisted,
    CaptureCaughtUp,
}

impl LifecycleStage {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::HookReceived => "hook_received",
            Self::RetrievalDecided => "retrieval_decided",
            Self::ReplyFlushed => "reply_flushed",
            Self::McpRequest => "mcp_request",
            Self::McpSucceeded => "mcp_succeeded",
            Self::McpFailed => "mcp_failed",
            Self::SessionEndPersisted => "session_end_persisted",
            Self::CaptureCaughtUp => "capture_caught_up",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "hook_received" => Self::HookReceived,
            "retrieval_decided" => Self::RetrievalDecided,
            "reply_flushed" => Self::ReplyFlushed,
            "mcp_request" => Self::McpRequest,
            "mcp_succeeded" => Self::McpSucceeded,
            "mcp_failed" => Self::McpFailed,
            "session_end_persisted" => Self::SessionEndPersisted,
            "capture_caught_up" => Self::CaptureCaughtUp,
            _ => bail!("unknown lifecycle stage {value:?}"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "state",
    content = "native_session_id"
)]
pub enum SessionAttribution {
    Attributed(String),
    Unattributed,
}

impl SessionAttribution {
    fn validate(&self) -> Result<()> {
        if let Self::Attributed(session_id) = self {
            ensure!(!session_id.trim().is_empty(), "native session id is empty");
        }
        Ok(())
    }

    fn database_values(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Attributed(session_id) => ("attributed", Some(session_id)),
            Self::Unattributed => ("unattributed", None),
        }
    }

    fn from_database(state: &str, native_session_id: Option<String>) -> Result<Self> {
        match (state, native_session_id) {
            ("attributed", Some(session_id)) if !session_id.trim().is_empty() => {
                Ok(Self::Attributed(session_id))
            }
            ("unattributed", None) => Ok(Self::Unattributed),
            _ => bail!("invalid stored session attribution"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct LifecycleEvent {
    pub event_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub harness: Harness,
    pub session: SessionAttribution,
    pub correlation_id: Option<String>,
    pub channel: LifecycleChannel,
    pub stage: LifecycleStage,
    pub occurred_at: time::OffsetDateTime,
    pub detail: serde_json::Value,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalOutcome {
    Delivered,
    HealthySilence,
    Dropped,
    Failed,
    NotRequested,
}

impl RetrievalOutcome {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::HealthySilence => "healthy_silence",
            Self::Dropped => "dropped",
            Self::Failed => "failed",
            Self::NotRequested => "not_requested",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "delivered" => Self::Delivered,
            "healthy_silence" => Self::HealthySilence,
            "dropped" => Self::Dropped,
            "failed" => Self::Failed,
            "not_requested" => Self::NotRequested,
            _ => bail!("unknown retrieval outcome {value:?}"),
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalReasonCode {
    Selected,
    ShortPrompt,
    MissingSessionId,
    SessionMemoryCap,
    NoRelevantCandidate,
    BudgetDrop,
    RetrievalError,
    Timeout,
    NotRequested,
}

impl RetrievalReasonCode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::ShortPrompt => "short_prompt",
            Self::MissingSessionId => "missing_session_id",
            Self::SessionMemoryCap => "session_memory_cap",
            Self::NoRelevantCandidate => "no_relevant_candidate",
            Self::BudgetDrop => "budget_drop",
            Self::RetrievalError => "retrieval_error",
            Self::Timeout => "timeout",
            Self::NotRequested => "not_requested",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        Ok(match value {
            "selected" => Self::Selected,
            "short_prompt" => Self::ShortPrompt,
            "missing_session_id" => Self::MissingSessionId,
            "session_memory_cap" => Self::SessionMemoryCap,
            "no_relevant_candidate" => Self::NoRelevantCandidate,
            "budget_drop" => Self::BudgetDrop,
            "retrieval_error" => Self::RetrievalError,
            "timeout" => Self::Timeout,
            "not_requested" => Self::NotRequested,
            _ => bail!("unknown retrieval reason code {value:?}"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RetrievalDecision {
    pub decision_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub harness: Harness,
    pub session: SessionAttribution,
    pub correlation_id: Option<String>,
    pub channel: LifecycleChannel,
    pub outcome: RetrievalOutcome,
    pub reason_code: RetrievalReasonCode,
    pub candidate_count: u32,
    pub selected_count: u32,
    pub dropped_count: u32,
    pub token_count: u32,
    pub latency_ms: u64,
    pub query_sha256: String,
    pub selected_evidence_ids: Vec<String>,
    pub occurred_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryQuery {
    pub project_id: ProjectId,
    pub session: Option<SessionAttribution>,
    pub start: time::OffsetDateTime,
    pub end: time::OffsetDateTime,
    pub limit: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct LifecycleFold {
    pub total: u64,
    pub stage_counts: BTreeMap<LifecycleStage, u64>,
    pub last_occurred_at: Option<time::OffsetDateTime>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RetrievalDecisionFold {
    pub total: u64,
    pub delivered: u64,
    pub healthy_silence: u64,
    pub dropped: u64,
    pub failed: u64,
    pub not_requested: u64,
    pub last_occurred_at: Option<time::OffsetDateTime>,
}

pub fn fold_lifecycle(events: &[LifecycleEvent]) -> LifecycleFold {
    let mut fold = LifecycleFold::default();
    for event in events {
        fold.total = fold.total.saturating_add(1);
        *fold.stage_counts.entry(event.stage.clone()).or_insert(0) += 1;
        fold.last_occurred_at = Some(
            fold.last_occurred_at
                .map_or(event.occurred_at, |current| current.max(event.occurred_at)),
        );
    }
    fold
}

pub fn fold_retrieval_decisions(decisions: &[RetrievalDecision]) -> RetrievalDecisionFold {
    let mut fold = RetrievalDecisionFold::default();
    for decision in decisions {
        fold.total = fold.total.saturating_add(1);
        match decision.outcome {
            RetrievalOutcome::Delivered => fold.delivered = fold.delivered.saturating_add(1),
            RetrievalOutcome::HealthySilence => {
                fold.healthy_silence = fold.healthy_silence.saturating_add(1)
            }
            RetrievalOutcome::Dropped => fold.dropped = fold.dropped.saturating_add(1),
            RetrievalOutcome::Failed => fold.failed = fold.failed.saturating_add(1),
            RetrievalOutcome::NotRequested => {
                fold.not_requested = fold.not_requested.saturating_add(1)
            }
        }
        fold.last_occurred_at = Some(
            fold.last_occurred_at
                .map_or(decision.occurred_at, |current| {
                    current.max(decision.occurred_at)
                }),
        );
    }
    fold
}

impl EventLedger {
    pub fn record_lifecycle_event(&self, event: &LifecycleEvent) -> Result<bool> {
        ensure_project(self, event.project_id)?;
        event.session.validate()?;
        validate_correlation(event.correlation_id.as_deref())?;
        let (attribution, native_session_id) = event.session.database_values();
        let changed = self.connection.execute(
            r#"
            INSERT OR IGNORE INTO lifecycle_events(
                event_id, project_id, harness, session_attribution, native_session_id,
                correlation_id, channel, stage, occurred_at_ns, detail_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                event.event_id.to_string(),
                event.project_id.0.to_string(),
                event.harness.as_str(),
                attribution,
                native_session_id,
                event.correlation_id,
                event.channel.as_str(),
                event.stage.as_str(),
                timestamp_ns(event.occurred_at)?,
                serde_json::to_string(&event.detail)?,
            ],
        )?;
        if changed == 1 {
            return Ok(true);
        }
        let existing = self
            .lifecycle_event_by_id(event.event_id)?
            .context("lifecycle event disappeared after duplicate insert")?;
        ensure!(existing == *event, "lifecycle event id conflict");
        Ok(false)
    }

    pub fn record_retrieval_decision(&self, decision: &RetrievalDecision) -> Result<bool> {
        ensure_project(self, decision.project_id)?;
        decision.session.validate()?;
        validate_correlation(decision.correlation_id.as_deref())?;
        ensure!(
            decision
                .selected_count
                .saturating_add(decision.dropped_count)
                <= decision.candidate_count,
            "selected plus dropped retrieval counts exceed candidates"
        );
        ensure!(
            decision.query_sha256.len() == 64
                && decision
                    .query_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "retrieval query hash is not lowercase SHA-256"
        );
        let (attribution, native_session_id) = decision.session.database_values();
        let changed = self.connection.execute(
            r#"
            INSERT OR IGNORE INTO retrieval_decisions(
                decision_id, project_id, harness, session_attribution, native_session_id,
                correlation_id, channel, outcome, reason_code, candidate_count,
                selected_count, dropped_count, token_count, latency_ms, query_sha256,
                selected_evidence_json, occurred_at_ns
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17
            )
            "#,
            params![
                decision.decision_id.to_string(),
                decision.project_id.0.to_string(),
                decision.harness.as_str(),
                attribution,
                native_session_id,
                decision.correlation_id,
                decision.channel.as_str(),
                decision.outcome.as_str(),
                decision.reason_code.as_str(),
                i64::from(decision.candidate_count),
                i64::from(decision.selected_count),
                i64::from(decision.dropped_count),
                i64::from(decision.token_count),
                i64::try_from(decision.latency_ms).context("retrieval latency exceeds SQLite")?,
                decision.query_sha256,
                serde_json::to_string(&decision.selected_evidence_ids)?,
                timestamp_ns(decision.occurred_at)?,
            ],
        )?;
        if changed == 1 {
            return Ok(true);
        }
        let existing = self
            .retrieval_decision_by_id(decision.decision_id)?
            .context("retrieval decision disappeared after duplicate insert")?;
        ensure!(existing == *decision, "retrieval decision id conflict");
        Ok(false)
    }

    pub fn lifecycle_events(&self, query: &TelemetryQuery) -> Result<Vec<LifecycleEvent>> {
        validate_query(self, query)?;
        let (attribution, native_session_id) = query_session_values(query.session.as_ref());
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, project_id, harness, session_attribution, native_session_id,
                   correlation_id, channel, stage, occurred_at_ns, detail_json
            FROM lifecycle_events
            WHERE project_id = ?1 AND occurred_at_ns >= ?2 AND occurred_at_ns <= ?3
              AND (?4 IS NULL OR session_attribution = ?4)
              AND (?5 IS NULL OR native_session_id = ?5)
            ORDER BY occurred_at_ns ASC, event_id ASC
            LIMIT ?6
            "#,
        )?;
        let rows = statement.query_map(
            params![
                query.project_id.0.to_string(),
                timestamp_ns(query.start)?,
                timestamp_ns(query.end)?,
                attribution,
                native_session_id,
                i64::from(query.limit),
            ],
            lifecycle_row,
        )?;
        rows.map(|row| parse_lifecycle_row(row?)).collect()
    }

    pub fn retrieval_decisions(&self, query: &TelemetryQuery) -> Result<Vec<RetrievalDecision>> {
        validate_query(self, query)?;
        let (attribution, native_session_id) = query_session_values(query.session.as_ref());
        let mut statement = self.connection.prepare(
            r#"
            SELECT decision_id, project_id, harness, session_attribution, native_session_id,
                   correlation_id, channel, outcome, reason_code, candidate_count,
                   selected_count, dropped_count, token_count, latency_ms, query_sha256,
                   selected_evidence_json, occurred_at_ns
            FROM retrieval_decisions
            WHERE project_id = ?1 AND occurred_at_ns >= ?2 AND occurred_at_ns <= ?3
              AND (?4 IS NULL OR session_attribution = ?4)
              AND (?5 IS NULL OR native_session_id = ?5)
            ORDER BY occurred_at_ns ASC, decision_id ASC
            LIMIT ?6
            "#,
        )?;
        let rows = statement.query_map(
            params![
                query.project_id.0.to_string(),
                timestamp_ns(query.start)?,
                timestamp_ns(query.end)?,
                attribution,
                native_session_id,
                i64::from(query.limit),
            ],
            retrieval_row,
        )?;
        rows.map(|row| parse_retrieval_row(row?)).collect()
    }

    fn lifecycle_event_by_id(&self, event_id: uuid::Uuid) -> Result<Option<LifecycleEvent>> {
        self.connection
            .query_row(
                r#"
                SELECT event_id, project_id, harness, session_attribution, native_session_id,
                       correlation_id, channel, stage, occurred_at_ns, detail_json
                FROM lifecycle_events WHERE event_id = ?1
                "#,
                [event_id.to_string()],
                lifecycle_row,
            )
            .optional()?
            .map(parse_lifecycle_row)
            .transpose()
    }

    fn retrieval_decision_by_id(
        &self,
        decision_id: uuid::Uuid,
    ) -> Result<Option<RetrievalDecision>> {
        self.connection
            .query_row(
                r#"
                SELECT decision_id, project_id, harness, session_attribution, native_session_id,
                       correlation_id, channel, outcome, reason_code, candidate_count,
                       selected_count, dropped_count, token_count, latency_ms, query_sha256,
                       selected_evidence_json, occurred_at_ns
                FROM retrieval_decisions WHERE decision_id = ?1
                "#,
                [decision_id.to_string()],
                retrieval_row,
            )
            .optional()?
            .map(parse_retrieval_row)
            .transpose()
    }
}

type RawLifecycle = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    i64,
    String,
);

fn lifecycle_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawLifecycle> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn parse_lifecycle_row(raw: RawLifecycle) -> Result<LifecycleEvent> {
    Ok(LifecycleEvent {
        event_id: uuid::Uuid::parse_str(&raw.0)?,
        project_id: ProjectId(uuid::Uuid::parse_str(&raw.1)?),
        harness: parse_harness(&raw.2),
        session: SessionAttribution::from_database(&raw.3, raw.4)?,
        correlation_id: raw.5,
        channel: LifecycleChannel::from_str(&raw.6)?,
        stage: LifecycleStage::from_str(&raw.7)?,
        occurred_at: from_ns(raw.8)?,
        detail: serde_json::from_str(&raw.9)?,
    })
}

type RawRetrieval = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
    i64,
);

fn retrieval_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRetrieval> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
    ))
}

fn parse_retrieval_row(raw: RawRetrieval) -> Result<RetrievalDecision> {
    Ok(RetrievalDecision {
        decision_id: uuid::Uuid::parse_str(&raw.0)?,
        project_id: ProjectId(uuid::Uuid::parse_str(&raw.1)?),
        harness: parse_harness(&raw.2),
        session: SessionAttribution::from_database(&raw.3, raw.4)?,
        correlation_id: raw.5,
        channel: LifecycleChannel::from_str(&raw.6)?,
        outcome: RetrievalOutcome::from_str(&raw.7)?,
        reason_code: RetrievalReasonCode::from_str(&raw.8)?,
        candidate_count: u32::try_from(raw.9).context("stored candidate count is invalid")?,
        selected_count: u32::try_from(raw.10).context("stored selected count is invalid")?,
        dropped_count: u32::try_from(raw.11).context("stored dropped count is invalid")?,
        token_count: u32::try_from(raw.12).context("stored token count is invalid")?,
        latency_ms: u64::try_from(raw.13).context("stored retrieval latency is invalid")?,
        query_sha256: raw.14,
        selected_evidence_ids: serde_json::from_str(&raw.15)?,
        occurred_at: from_ns(raw.16)?,
    })
}

fn ensure_project(ledger: &EventLedger, project_id: ProjectId) -> Result<()> {
    ensure!(
        project_id == ledger.project_scope,
        "telemetry belongs to another project"
    );
    Ok(())
}

fn validate_query(ledger: &EventLedger, query: &TelemetryQuery) -> Result<()> {
    ensure_project(ledger, query.project_id)?;
    ensure!(
        query.start <= query.end,
        "telemetry query time range is inverted"
    );
    ensure!(
        (1..=10_000).contains(&query.limit),
        "telemetry query limit is invalid"
    );
    if let Some(session) = &query.session {
        session.validate()?;
    }
    Ok(())
}

fn validate_correlation(correlation_id: Option<&str>) -> Result<()> {
    if let Some(correlation_id) = correlation_id {
        ensure!(!correlation_id.trim().is_empty(), "correlation id is empty");
    }
    Ok(())
}

fn query_session_values(session: Option<&SessionAttribution>) -> (Option<&str>, Option<&str>) {
    session.map_or((None, None), |session| {
        let (attribution, native_session_id) = session.database_values();
        (Some(attribution), native_session_id)
    })
}

fn parse_harness(value: &str) -> Harness {
    match value {
        "claude-code" => Harness::ClaudeCode,
        "codex" => Harness::Codex,
        "hermes" => Harness::Hermes,
        other => Harness::Other(other.to_owned()),
    }
}

fn from_ns(value: i64) -> Result<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value)).map_err(Into::into)
}
