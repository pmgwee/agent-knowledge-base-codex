use anyhow::{Result, bail};
use brain_domain::{Authority, MemoryKind, MemoryStatus, ProjectId, WorktreeId};
use rusqlite::{Row, params};

use crate::EventLedger;
use crate::cursor::timestamp_ns;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const MAX_QUERY_TERMS: usize = 32;
const MAX_TERM_CHARACTERS: usize = 64;
const LATE_OBSERVATION_THRESHOLD: time::Duration = time::Duration::minutes(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSource {
    Event,
    Memory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSourceFilter {
    All,
    Events,
    Memories,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeRange {
    pub start: time::OffsetDateTime,
    pub end: time::OffsetDateTime,
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub project_id: ProjectId,
    pub text: Option<String>,
    pub occurred: Option<TimeRange>,
    pub as_of: Option<time::OffsetDateTime>,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub native_session_id: Option<String>,
    pub source_filter: SearchSourceFilter,
    pub limit: usize,
}

impl SearchQuery {
    pub fn text(project_id: ProjectId, text: impl Into<String>) -> Self {
        Self {
            project_id,
            text: Some(text.into()),
            occurred: None,
            as_of: None,
            worktree_id: None,
            task_id: None,
            native_session_id: None,
            source_filter: SearchSourceFilter::All,
            limit: DEFAULT_LIMIT,
        }
    }

    pub fn between(
        project_id: ProjectId,
        start: time::OffsetDateTime,
        end: time::OffsetDateTime,
    ) -> Self {
        let mut query = Self::text(project_id, "");
        query.text = None;
        query.occurred = Some(TimeRange { start, end });
        query
    }

    pub fn last_day(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(1), now)
    }

    pub fn last_week(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(7), now)
    }

    pub fn last_month(project_id: ProjectId, now: time::OffsetDateTime) -> Self {
        Self::between(project_id, now - time::Duration::days(30), now)
    }

    pub fn as_of(mut self, as_of: time::OffsetDateTime) -> Self {
        self.as_of = Some(as_of);
        self
    }

    pub fn for_worktree(mut self, worktree_id: WorktreeId) -> Self {
        self.worktree_id = Some(worktree_id);
        self
    }

    pub fn for_task(mut self, task_id: uuid::Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn for_session(mut self, native_session_id: impl Into<String>) -> Self {
        self.native_session_id = Some(native_session_id.into());
        self
    }

    pub fn events_only(mut self) -> Self {
        self.source_filter = SearchSourceFilter::Events;
        self
    }

    pub fn memories_only(mut self) -> Self {
        self.source_filter = SearchSourceFilter::Memories;
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.min(MAX_LIMIT);
        self
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub source: SearchSource,
    pub source_id: uuid::Uuid,
    pub memory_id: Option<uuid::Uuid>,
    pub project_id: ProjectId,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub native_session_id: Option<String>,
    pub kind: Option<MemoryKind>,
    pub title: String,
    pub text: String,
    pub path: Option<String>,
    pub occurred_at: time::OffsetDateTime,
    pub observed_at: time::OffsetDateTime,
    pub late_observation: bool,
    pub authority: Option<Authority>,
    pub status: Option<MemoryStatus>,
    pub evidence_count: usize,
    pub bm25_score: f64,
}

impl EventLedger {
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        if query.project_id != self.project_scope {
            bail!(
                "project scope {} cannot search ledger scoped to {}",
                query.project_id.0,
                self.project_scope.0
            );
        }
        if let Some(range) = query.occurred
            && range.start >= range.end
        {
            bail!("search time range must have start before end");
        }
        if query.limit == 0 {
            return Ok(Vec::new());
        }
        if query.text.is_some()
            && match_expression(query.project_id, query.text.as_deref()).is_none()
        {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        if query.source_filter != SearchSourceFilter::Memories {
            hits.extend(self.search_events(query)?);
        }
        if query.source_filter != SearchSourceFilter::Events {
            hits.extend(self.search_memories(query)?);
        }
        hits.sort_by(|left, right| {
            right
                .bm25_score
                .total_cmp(&left.bm25_score)
                .then_with(|| right.occurred_at.cmp(&left.occurred_at))
                .then_with(|| right.source_id.cmp(&left.source_id))
        });
        hits.truncate(query.limit.min(MAX_LIMIT));
        Ok(hits)
    }

    pub fn explain_text_search(&self, query: &SearchQuery) -> Result<Vec<String>> {
        if query.project_id != self.project_scope {
            bail!("project scope does not match this ledger");
        }
        let Some(match_value) = match_expression(query.project_id, query.text.as_deref()) else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            r#"
            EXPLAIN QUERY PLAN
            SELECT e.event_id
            FROM event_search
            JOIN events e ON e.rowid = event_search.rowid
            WHERE e.project_id = ?1 AND event_search MATCH ?2
            LIMIT 50
            "#,
        )?;
        let rows = statement.query_map(
            params![query.project_id.0.to_string(), match_value],
            |row| row.get(3),
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn search_events(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let project = query.project_id.0.to_string();
        let start = query
            .occurred
            .map(|range| timestamp_ns(range.start))
            .transpose()?;
        let mut end = query
            .occurred
            .map(|range| timestamp_ns(range.end))
            .transpose()?;
        if let Some(as_of) = query.as_of {
            let as_of = timestamp_ns(as_of)?;
            end = Some(end.map_or(as_of, |end| end.min(as_of)));
        }
        let worktree = query.worktree_id.map(|id| id.0.to_string());
        let task = query.task_id.map(|id| id.to_string());
        let limit = i64::try_from(query.limit.min(MAX_LIMIT))?;
        let text_match = match_expression(query.project_id, query.text.as_deref());
        let (sql, match_value) = if let Some(match_value) = text_match {
            (
                r#"
                SELECT e.event_id, e.worktree_id, e.task_id, e.native_session_id,
                       e.event_type, e.occurred_at_ns, e.observed_at_ns,
                       coalesce(
                           json_extract(e.payload_json, '$.path'),
                           json_extract(e.payload_json, '$.file_path'),
                           e.source_locator
                       ), e.payload_json,
                       -bm25(event_search, 0.0, 1.0, 2.0, 2.0, 0.5)
                FROM event_search
                JOIN events e ON e.rowid = event_search.rowid
                WHERE e.project_id = ?1 AND event_search MATCH ?2
                  AND (?3 IS NULL OR e.occurred_at_ns >= ?3)
                  AND (?4 IS NULL OR e.occurred_at_ns < ?4)
                  AND (?5 IS NULL OR e.worktree_id = ?5)
                  AND (?6 IS NULL OR e.task_id = ?6)
                  AND (?7 IS NULL OR e.native_session_id = ?7)
                ORDER BY bm25(event_search), e.occurred_at_ns DESC
                LIMIT ?8
                "#,
                Some(match_value),
            )
        } else {
            (
                r#"
                SELECT e.event_id, e.worktree_id, e.task_id, e.native_session_id,
                       e.event_type, e.occurred_at_ns, e.observed_at_ns,
                       coalesce(
                           json_extract(e.payload_json, '$.path'),
                           json_extract(e.payload_json, '$.file_path'),
                           e.source_locator
                       ), e.payload_json, 0.0
                FROM events e
                WHERE e.project_id = ?1
                  AND (?3 IS NULL OR e.occurred_at_ns >= ?3)
                  AND (?4 IS NULL OR e.occurred_at_ns < ?4)
                  AND (?5 IS NULL OR e.worktree_id = ?5)
                  AND (?6 IS NULL OR e.task_id = ?6)
                  AND (?7 IS NULL OR e.native_session_id = ?7)
                ORDER BY e.occurred_at_ns DESC
                LIMIT ?8
                "#,
                None,
            )
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(
            params![
                project,
                match_value,
                start,
                end,
                worktree,
                task,
                query.native_session_id,
                limit
            ],
            parse_event_hit,
        )?;
        rows.map(|row| row?.into_hit(query.project_id)).collect()
    }

    fn search_memories(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let project = query.project_id.0.to_string();
        let start = query
            .occurred
            .map(|range| timestamp_ns(range.start))
            .transpose()?;
        let end = query
            .occurred
            .map(|range| timestamp_ns(range.end))
            .transpose()?;
        let as_of = query
            .as_of
            .map(timestamp_ns)
            .transpose()?
            .unwrap_or(i64::MAX);
        let worktree = query.worktree_id.map(|id| id.0.to_string());
        let task = query.task_id.map(|id| id.to_string());
        let limit = i64::try_from(query.limit.min(MAX_LIMIT))?;
        let text_match = match_expression(query.project_id, query.text.as_deref());
        let (sql, match_value) = if let Some(match_value) = text_match {
            (
                r#"
                SELECT v.version_id, v.memory_id, v.worktree_id, v.task_id,
                       r.kind, v.title, v.content, v.valid_from_ns, v.recorded_at_ns,
                       v.authority, v.status,
                       (SELECT COUNT(*) FROM memory_evidence me WHERE me.version_id = v.version_id),
                       -bm25(memory_search, 0.0, 2.0, 1.0, 1.5, 1.5, 0.5),
                       r.projection_path
                FROM memory_search
                JOIN memory_versions v ON v.rowid = memory_search.rowid
                JOIN memory_records r ON r.memory_id = v.memory_id
                WHERE r.project_id = ?1 AND memory_search MATCH ?2
                  AND v.valid_from_ns <= ?5
                  AND (v.valid_to_ns IS NULL OR v.valid_to_ns > ?5)
                  AND (?3 IS NULL OR v.valid_from_ns >= ?3)
                  AND (?4 IS NULL OR v.valid_from_ns < ?4)
                  AND (?6 IS NULL OR v.worktree_id = ?6)
                  AND (?7 IS NULL OR v.task_id = ?7)
                  AND v.status NOT IN ('invalid', 'superseded')
                  AND NOT EXISTS (
                      SELECT 1
                      FROM memory_supersession s
                      JOIN memory_versions newer ON newer.version_id = s.version_id
                      WHERE s.superseded_version_id = v.version_id
                        AND newer.valid_from_ns <= ?5
                  )
                ORDER BY bm25(memory_search), v.valid_from_ns DESC
                LIMIT ?8
                "#,
                Some(match_value),
            )
        } else {
            (
                r#"
                SELECT v.version_id, v.memory_id, v.worktree_id, v.task_id,
                       r.kind, v.title, v.content, v.valid_from_ns, v.recorded_at_ns,
                       v.authority, v.status,
                       (SELECT COUNT(*) FROM memory_evidence me WHERE me.version_id = v.version_id),
                       0.0, r.projection_path
                FROM memory_versions v
                JOIN memory_records r ON r.memory_id = v.memory_id
                WHERE r.project_id = ?1
                  AND v.valid_from_ns <= ?5
                  AND (v.valid_to_ns IS NULL OR v.valid_to_ns > ?5)
                  AND (?3 IS NULL OR v.valid_from_ns >= ?3)
                  AND (?4 IS NULL OR v.valid_from_ns < ?4)
                  AND (?6 IS NULL OR v.worktree_id = ?6)
                  AND (?7 IS NULL OR v.task_id = ?7)
                  AND v.status NOT IN ('invalid', 'superseded')
                  AND NOT EXISTS (
                      SELECT 1
                      FROM memory_supersession s
                      JOIN memory_versions newer ON newer.version_id = s.version_id
                      WHERE s.superseded_version_id = v.version_id
                        AND newer.valid_from_ns <= ?5
                  )
                ORDER BY v.valid_from_ns DESC
                LIMIT ?8
                "#,
                None,
            )
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(
            params![
                project,
                match_value,
                start,
                end,
                as_of,
                worktree,
                task,
                limit
            ],
            parse_memory_hit,
        )?;
        rows.map(|row| row?.into_hit(query.project_id)).collect()
    }
}

fn match_expression(project_id: ProjectId, text: Option<&str>) -> Option<String> {
    let terms = text?
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .take(MAX_QUERY_TERMS)
        .map(|term| term.chars().take(MAX_TERM_CHARACTERS).collect::<String>())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return None;
    }
    let scope = format!("p{}", project_id.0.simple());
    Some(format!(
        "scope_token:\"{scope}\" AND ({})",
        terms.join(" AND ")
    ))
}

struct RawEventHit {
    source_id: String,
    worktree_id: String,
    task_id: Option<String>,
    native_session_id: String,
    event_type: String,
    occurred_at_ns: i64,
    observed_at_ns: i64,
    source_locator: String,
    payload_json: String,
    bm25_score: f64,
}

fn parse_event_hit(row: &Row<'_>) -> rusqlite::Result<RawEventHit> {
    Ok(RawEventHit {
        source_id: row.get(0)?,
        worktree_id: row.get(1)?,
        task_id: row.get(2)?,
        native_session_id: row.get(3)?,
        event_type: row.get(4)?,
        occurred_at_ns: row.get(5)?,
        observed_at_ns: row.get(6)?,
        source_locator: row.get(7)?,
        payload_json: row.get(8)?,
        bm25_score: row.get(9)?,
    })
}

impl RawEventHit {
    fn into_hit(self, project_id: ProjectId) -> Result<SearchHit> {
        let occurred_at = from_ns(self.occurred_at_ns)?;
        let observed_at = from_ns(self.observed_at_ns)?;
        Ok(SearchHit {
            source: SearchSource::Event,
            source_id: uuid::Uuid::parse_str(&self.source_id)?,
            memory_id: None,
            project_id,
            worktree_id: Some(WorktreeId(uuid::Uuid::parse_str(&self.worktree_id)?)),
            task_id: self
                .task_id
                .map(|id| uuid::Uuid::parse_str(&id))
                .transpose()?,
            native_session_id: Some(self.native_session_id),
            kind: None,
            title: self.event_type,
            text: event_text(&self.payload_json),
            path: Some(self.source_locator),
            occurred_at,
            observed_at,
            late_observation: observed_at - occurred_at > LATE_OBSERVATION_THRESHOLD,
            authority: Some(Authority::RawMechanicalEvidence),
            status: None,
            evidence_count: 1,
            bm25_score: self.bm25_score,
        })
    }
}

struct RawMemoryHit {
    version_id: String,
    memory_id: String,
    worktree_id: Option<String>,
    task_id: Option<String>,
    kind: String,
    title: String,
    content: String,
    valid_from_ns: i64,
    recorded_at_ns: i64,
    authority: String,
    status: String,
    evidence_count: i64,
    bm25_score: f64,
    projection_path: String,
}

fn parse_memory_hit(row: &Row<'_>) -> rusqlite::Result<RawMemoryHit> {
    Ok(RawMemoryHit {
        version_id: row.get(0)?,
        memory_id: row.get(1)?,
        worktree_id: row.get(2)?,
        task_id: row.get(3)?,
        kind: row.get(4)?,
        title: row.get(5)?,
        content: row.get(6)?,
        valid_from_ns: row.get(7)?,
        recorded_at_ns: row.get(8)?,
        authority: row.get(9)?,
        status: row.get(10)?,
        evidence_count: row.get(11)?,
        bm25_score: row.get(12)?,
        projection_path: row.get(13)?,
    })
}

impl RawMemoryHit {
    fn into_hit(self, project_id: ProjectId) -> Result<SearchHit> {
        let occurred_at = from_ns(self.valid_from_ns)?;
        let observed_at = from_ns(self.recorded_at_ns)?;
        Ok(SearchHit {
            source: SearchSource::Memory,
            source_id: uuid::Uuid::parse_str(&self.version_id)?,
            memory_id: Some(uuid::Uuid::parse_str(&self.memory_id)?),
            project_id,
            worktree_id: self
                .worktree_id
                .map(|id| uuid::Uuid::parse_str(&id).map(WorktreeId))
                .transpose()?,
            task_id: self
                .task_id
                .map(|id| uuid::Uuid::parse_str(&id))
                .transpose()?,
            native_session_id: None,
            kind: MemoryKind::from_name(&self.kind),
            title: self.title,
            text: self.content,
            path: Some(self.projection_path),
            occurred_at,
            observed_at,
            late_observation: observed_at - occurred_at > LATE_OBSERVATION_THRESHOLD,
            authority: Authority::from_name(&self.authority),
            status: MemoryStatus::from_name(&self.status),
            evidence_count: usize::try_from(self.evidence_count)?,
            bm25_score: self.bm25_score,
        })
    }
}

fn event_text(payload_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload_json) else {
        return String::new();
    };
    for key in ["content", "text", "summary", "message", "path", "file_path"] {
        if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
            return text.to_owned();
        }
    }
    value.to_string()
}

fn from_ns(value: i64) -> Result<time::OffsetDateTime> {
    Ok(time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(value),
    )?)
}
