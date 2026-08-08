use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Result, bail};
use brain_domain::{Authority, Harness, MemoryKind, MemoryStatus, ProjectId, WorktreeId};
use rusqlite::{Row, params};

use crate::EventLedger;
use crate::cursor::timestamp_ns;
use crate::embedding::shared_embedder;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const MAX_QUERY_TERMS: usize = 32;
const MAX_TERM_CHARACTERS: usize = 64;
const MAX_SEARCH_CACHE_ENTRIES: usize = 128;
const MAX_SEARCH_CACHE_BYTES: usize = 16 * 1024 * 1024;
const LATE_OBSERVATION_THRESHOLD: time::Duration = time::Duration::minutes(5);

/// Reciprocal Rank Fusion's rank offset.
///
/// `60` is the value from Cormack, Clarke and Buettcher's original paper and the one every
/// subsequent comparison uses; keeping it means our fusion is the fusion everyone else measured,
/// not a variant of it. Its job is to flatten the head: without it the first result of a channel
/// is worth twice the second, which lets one confident-but-wrong channel dominate.
const RRF_K: f64 = 60.0;

/// Channel weights. Vector outweighs keyword because the queries keyword search already answers
/// are the ones it answers well — the fusion exists for the queries it cannot, where the evidence
/// shares no vocabulary with the question. These are a starting point from the literature, not a
/// tuned result on this corpus, and the benchmark is what will move them.
const BM25_WEIGHT: f64 = 0.4;
const VECTOR_WEIGHT: f64 = 0.6;

/// The graph channel's weight, held well under the retrieval channels on purpose: an evidence
/// link says two documents are related, not that either answers the question.
const GRAPH_WEIGHT: f64 = 0.2;

/// How deep each channel is fetched before fusion.
///
/// Fusion can only reorder what it is given, so a channel fetched to the caller's limit can never
/// rescue a document the other channel ranked 40th. Depth is where recall comes from; the limit
/// is only what survives.
const FUSION_DEPTH: usize = 100;

/// Most results from any one session before the rest are pushed behind other sessions.
///
/// A single long session can fill an entire answer with near-duplicate turns, which is a worse
/// answer than the same budget spread across the sessions that actually differ. Overflow is
/// demoted rather than dropped: a capped hit still appears, just after the others, so this can
/// reorder recall but never lose it.
const MAX_HITS_PER_SESSION: usize = 3;

/// Most documents a graph expansion may append.
const MAX_EXPANDED_HITS: usize = 20;

/// How many fused results are used as expansion seeds.
const EXPANSION_SEEDS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSource {
    Event,
    Memory,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SearchSourceFilter {
    All,
    Events,
    Memories,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimeRange {
    pub start: time::OffsetDateTime,
    pub end: time::OffsetDateTime,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SearchQuery {
    pub project_id: ProjectId,
    pub text: Option<String>,
    pub occurred: Option<TimeRange>,
    pub as_of: Option<time::OffsetDateTime>,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub native_session_id: Option<String>,
    pub source_filter: SearchSourceFilter,
    /// Cap how many results one session may contribute. On by default: a caller asking a
    /// question wants the answer, and the answer is rarely the same session five times.
    /// Off is for callers that asked *about* a session, and for the benchmark, which needs
    /// to attribute a change to fusion rather than to this.
    pub diversify_sessions: bool,
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
            diversify_sessions: true,
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

    pub fn without_session_diversity(mut self) -> Self {
        self.diversify_sessions = false;
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
    pub harness: Option<Harness>,
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
    /// This hit's BM25 score, or 0.0 when no keyword channel matched it.
    pub bm25_score: f64,
    /// The score this ledger ordered by: the fused score when channels were fused, and the
    /// BM25 score when only keyword search ran. Re-ranking callers must use this — reading
    /// `bm25_score` instead scores every vector-only hit as zero and quietly undoes fusion.
    pub rank_score: f64,
}

#[derive(Default)]
pub(crate) struct SearchCache {
    pub(crate) data_version: Option<i64>,
    pub(crate) entries: HashMap<SearchQuery, Vec<SearchHit>>,
    pub(crate) estimated_bytes: usize,
}

impl SearchCache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.estimated_bytes = 0;
    }
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
        let data_version = self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))?;
        {
            let mut cache = self.search_cache.borrow_mut();
            if cache.data_version != Some(data_version) {
                cache.clear();
                cache.data_version = Some(data_version);
            }
            if let Some(hits) = cache.entries.get(query) {
                return Ok(hits.clone());
            }
        }
        let hits = self.ranked_hits(query)?;
        let entry_bytes = estimated_cache_entry_bytes(query, &hits);
        if entry_bytes <= MAX_SEARCH_CACHE_BYTES {
            let mut cache = self.search_cache.borrow_mut();
            if cache.entries.len() >= MAX_SEARCH_CACHE_ENTRIES
                || cache.estimated_bytes.saturating_add(entry_bytes) > MAX_SEARCH_CACHE_BYTES
            {
                cache.clear();
            }
            cache.estimated_bytes = cache.estimated_bytes.saturating_add(entry_bytes);
            cache.entries.insert(query.clone(), hits.clone());
        }
        Ok(hits)
    }

    /// Turn on the vector channel, loading the model from `brain_home` if one is installed.
    ///
    /// Returns whether vector search is now active. A brain with no model on disk returns `false`
    /// and keeps searching by keyword, which is this feature's whole contract: it may add recall,
    /// it may never take any away. Callers with no brain home — tests, fixtures, anything working
    /// on a bare ledger — simply never call this and stay on the keyword path.
    pub fn enable_vector_search(&mut self, brain_home: &Path) -> bool {
        self.embedder = shared_embedder(brain_home);
        // Anything cached was computed before this channel existed.
        self.search_cache.borrow_mut().clear();
        self.embedder.is_some()
    }

    pub fn vector_search_enabled(&self) -> bool {
        self.embedder.is_some()
    }

    /// Retrieve from every available channel, fuse, diversify, truncate.
    ///
    /// The channels are deliberately unequal in kind. Keyword and vector are *retrieval*: each
    /// answers the query independently. The graph channel is *association*, defined over what
    /// those two found — so it cannot be gathered in the same pass, which is why fusion runs
    /// twice rather than once over three lists.
    fn ranked_hits(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
        let limit = query.limit.min(MAX_LIMIT);
        // Depth is where recall comes from; the limit is only what survives. It costs nothing
        // when there is a single channel, so it is paid only when there is something to fuse.
        let depth = if self.embedder.is_some() {
            FUSION_DEPTH.max(limit)
        } else {
            limit
        };

        let mut keyword = Vec::new();
        if query.source_filter != SearchSourceFilter::Memories {
            keyword.extend(self.search_events(query, depth, None)?);
        }
        if query.source_filter != SearchSourceFilter::Events {
            keyword.extend(self.search_memories(query, depth, None)?);
        }
        keyword.sort_by(|left, right| {
            right
                .bm25_score
                .total_cmp(&left.bm25_score)
                .then_with(|| right.occurred_at.cmp(&left.occurred_at))
                .then_with(|| right.source_id.cmp(&left.source_id))
        });
        keyword.truncate(depth);

        let mut channels = vec![(BM25_WEIGHT, keyword)];
        let vector = self.vector_channel(query, depth)?;
        if !vector.is_empty() {
            channels.push((VECTOR_WEIGHT, vector));
        }

        // Fusing a single channel is the identity: RRF's score falls strictly with rank, so the
        // input order survives untouched. That is what keeps a brain with no model on exactly the
        // path it was measured on.
        let seeds = fuse(&channels);
        let expansion = self.expand_by_evidence(&seeds, query)?;
        let mut ranked = if expansion.is_empty() {
            seeds
        } else {
            channels.push((GRAPH_WEIGHT, expansion));
            fuse(&channels)
        };

        // A caller who named a session asked about that session; capping it would answer a
        // different question.
        if query.diversify_sessions && query.native_session_id.is_none() {
            diversify_by_session(&mut ranked);
        }
        ranked.truncate(limit);
        Ok(ranked)
    }

    /// The vector channel: one query embedding, scanned against both indexes.
    ///
    /// Events and memories are merged by similarity rather than concatenated, because the same
    /// model embedded both into the same space and their scores are directly comparable.
    /// Concatenating would rank every memory above every event regardless of which one answers
    /// the question.
    fn vector_channel(&self, query: &SearchQuery, depth: usize) -> Result<Vec<SearchHit>> {
        let Some(embedder) = self.embedder else {
            return Ok(Vec::new());
        };
        let Some(text) = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            // A pure time-range query has nothing to embed, and its ordering is chronological by
            // design — a similarity would have no meaning against it.
            return Ok(Vec::new());
        };
        let query_vector = match embedder.embed(text) {
            Ok(vector) => vector,
            Err(error) => {
                // Fail open. A model that cannot encode this one query is not a reason to fail a
                // search that keyword retrieval can still answer.
                tracing::warn!(%error, "query embedding failed; searching by keyword alone");
                return Ok(Vec::new());
            }
        };

        let mut scored: Vec<(f32, SearchHit)> = Vec::new();
        if query.source_filter != SearchSourceFilter::Memories {
            let found = self.search_events_by_vector(&query_vector, depth)?;
            let ids: Vec<uuid::Uuid> = found.iter().map(|hit| hit.event_id).collect();
            // Hydration re-applies the caller's filters — time range, worktree, task, session.
            // Skipping them would let this channel return rows the caller excluded.
            let mut bodies = by_source_id(self.search_events(query, depth, Some(&ids))?);
            for hit in found {
                if let Some(body) = bodies.remove(&hit.event_id) {
                    scored.push((hit.similarity, body));
                }
            }
        }
        if query.source_filter != SearchSourceFilter::Events {
            let found = self.search_by_vector(&query_vector, depth)?;
            let ids: Vec<uuid::Uuid> = found.iter().map(|hit| hit.version_id).collect();
            let mut bodies = by_source_id(self.search_memories(query, depth, Some(&ids))?);
            for hit in found {
                if let Some(body) = bodies.remove(&hit.version_id) {
                    scored.push((hit.similarity, body));
                }
            }
        }
        scored.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then_with(|| left.1.source_id.cmp(&right.1.source_id))
        });
        scored.truncate(depth);
        Ok(scored.into_iter().map(|(_, hit)| hit).collect())
    }

    /// One hop across `memory_evidence`, in both directions.
    ///
    /// This is the edge that makes the two layers one brain rather than two indexes: a memory
    /// that ranks brings the events it was derived from, and an event that ranks brings the
    /// memories that cite it. Both directions earn their place, for opposite reasons — the first
    /// answers "what is this claim based on", the second "what did we conclude from this".
    ///
    /// It is unmeasurable on LongMemEval-S, and that is worth stating rather than discovering:
    /// those ledgers hold events and never run consolidation, so there are no memories and hence
    /// no evidence edges at all. The benchmark number with this channel is the number without it.
    /// Its weight is held well below the retrieval channels for exactly that reason.
    fn expand_by_evidence(
        &self,
        seeds: &[SearchHit],
        query: &SearchQuery,
    ) -> Result<Vec<SearchHit>> {
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let mut seed_versions = Vec::new();
        let mut seed_events = Vec::new();
        for hit in seeds.iter().take(EXPANSION_SEEDS) {
            match hit.source {
                SearchSource::Memory => seed_versions.push(hit.source_id),
                SearchSource::Event => seed_events.push(hit.source_id),
            }
        }
        // Anything already ranked is not an expansion; it is what is being expanded from.
        let already: HashSet<uuid::Uuid> = seeds.iter().map(|hit| hit.source_id).collect();

        let mut expanded = Vec::new();
        if query.source_filter != SearchSourceFilter::Memories && !seed_versions.is_empty() {
            let cited = self.linked_ids(
                "SELECT event_id FROM memory_evidence \
                 WHERE version_id IN (SELECT value FROM json_each(?1))",
                &seed_versions,
                &already,
            )?;
            expanded.extend(self.search_events(query, MAX_EXPANDED_HITS, Some(&cited))?);
        }
        if query.source_filter != SearchSourceFilter::Events && !seed_events.is_empty() {
            let citing = self.linked_ids(
                "SELECT version_id FROM memory_evidence \
                 WHERE event_id IN (SELECT value FROM json_each(?1))",
                &seed_events,
                &already,
            )?;
            expanded.extend(self.search_memories(query, MAX_EXPANDED_HITS, Some(&citing))?);
        }
        // An expansion has no relevance score of its own — it arrived by association, not by
        // matching — so recency is the only honest order left to give it.
        expanded.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
        expanded.truncate(MAX_EXPANDED_HITS);
        Ok(expanded)
    }

    fn linked_ids(
        &self,
        sql: &str,
        ids: &[uuid::Uuid],
        exclude: &HashSet<uuid::Uuid>,
    ) -> Result<Vec<uuid::Uuid>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params![id_list(ids)], |row| row.get::<_, String>(0))?;
        let mut linked = Vec::new();
        for row in rows {
            let id = uuid::Uuid::parse_str(&row?)?;
            if !exclude.contains(&id) {
                linked.push(id);
            }
            if linked.len() >= MAX_EXPANDED_HITS {
                break;
            }
        }
        Ok(linked)
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

    /// Keyword-or-chronological retrieval over events.
    ///
    /// `restrict` narrows to an explicit id set, which is how the vector and graph channels get
    /// their result bodies: they decide *which* documents, this decides what a document looks
    /// like and — the part that matters — re-applies every filter the caller asked for. It is
    /// bound as a nullable parameter present in both statements rather than a third SQL variant,
    /// so there is exactly one place where an event's filters are written down.
    fn search_events(
        &self,
        query: &SearchQuery,
        depth: usize,
        restrict: Option<&[uuid::Uuid]>,
    ) -> Result<Vec<SearchHit>> {
        if restrict.is_some_and(<[uuid::Uuid]>::is_empty) {
            return Ok(Vec::new());
        }
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
        let limit = i64::try_from(depth.min(MAX_LIMIT))?;
        let restrict = restrict.map(id_list);
        // An id restriction replaces the keyword selector rather than narrowing it. Keeping the
        // MATCH would intersect the two, so a document the vector channel found *because* it
        // shares no vocabulary with the question would be dropped on its way back — silently
        // limiting that channel to documents keyword search had already matched, which is the one
        // thing it exists not to be limited to. Terms are OR-joined and include words like "I",
        // so the intersection is usually non-empty, and the bug would have hidden in plain sight.
        let text_match = if restrict.is_some() {
            None
        } else {
            match_expression(query.project_id, query.text.as_deref())
        };
        let (sql, match_value) = if let Some(match_value) = text_match {
            (
                r#"
                SELECT e.event_id, e.worktree_id, e.task_id, e.native_session_id,
                       e.harness, e.event_type, e.occurred_at_ns, e.observed_at_ns,
                       coalesce(nullif(c.path, ''),
                           json_extract(e.payload_json, '$.path'),
                           json_extract(e.payload_json, '$.file_path'),
                           e.source_locator
                       ), coalesce(nullif(c.search_text, ''), e.payload_json),
                       -bm25(event_search, 0.0, 1.0, 2.0, 2.0, 0.5)
                FROM event_search
                JOIN events e ON e.rowid = event_search.rowid
                LEFT JOIN event_segment_catalog c ON c.event_id = e.event_id
                WHERE e.project_id = ?1 AND event_search MATCH ?2
                  AND (?3 IS NULL OR e.occurred_at_ns >= ?3)
                  AND (?4 IS NULL OR e.occurred_at_ns < ?4)
                  AND (?5 IS NULL OR e.worktree_id = ?5)
                  AND (?6 IS NULL OR e.task_id = ?6)
                  AND (?7 IS NULL OR e.native_session_id = ?7)
                  AND (?9 IS NULL OR e.event_id IN (SELECT value FROM json_each(?9)))
                ORDER BY bm25(event_search), e.occurred_at_ns DESC
                LIMIT ?8
                "#,
                Some(match_value),
            )
        } else {
            (
                r#"
                SELECT e.event_id, e.worktree_id, e.task_id, e.native_session_id,
                       e.harness, e.event_type, e.occurred_at_ns, e.observed_at_ns,
                       coalesce(nullif(c.path, ''),
                           json_extract(e.payload_json, '$.path'),
                           json_extract(e.payload_json, '$.file_path'),
                           e.source_locator
                       ), coalesce(nullif(c.search_text, ''), e.payload_json), 0.0
                FROM events e
                LEFT JOIN event_segment_catalog c ON c.event_id = e.event_id
                WHERE e.project_id = ?1
                  AND (?3 IS NULL OR e.occurred_at_ns >= ?3)
                  AND (?4 IS NULL OR e.occurred_at_ns < ?4)
                  AND (?5 IS NULL OR e.worktree_id = ?5)
                  AND (?6 IS NULL OR e.task_id = ?6)
                  AND (?7 IS NULL OR e.native_session_id = ?7)
                  AND (?9 IS NULL OR e.event_id IN (SELECT value FROM json_each(?9)))
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
                limit,
                restrict
            ],
            parse_event_hit,
        )?;
        rows.map(|row| row?.into_hit(query.project_id)).collect()
    }

    /// Keyword-or-chronological retrieval over memory versions. `restrict` behaves exactly as
    /// it does for events, and matters more here: the as-of and supersession clauses below are
    /// what keep a superseded claim from being returned, and a channel that selected memories by
    /// id without them would resurrect retracted memories as if they were current.
    fn search_memories(
        &self,
        query: &SearchQuery,
        depth: usize,
        restrict: Option<&[uuid::Uuid]>,
    ) -> Result<Vec<SearchHit>> {
        if restrict.is_some_and(<[uuid::Uuid]>::is_empty) {
            return Ok(Vec::new());
        }
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
        let limit = i64::try_from(depth.min(MAX_LIMIT))?;
        let restrict = restrict.map(id_list);
        // As in `search_events`: a restriction replaces the keyword selector, it does not narrow
        // it. See the comment there for why intersecting the two is the failure that hides.
        let text_match = if restrict.is_some() {
            None
        } else {
            match_expression(query.project_id, query.text.as_deref())
        };
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
                  -- A withdrawn memory is gone as far as every reader is concerned. Filtered in
                  -- SQL rather than after the fact so it never occupies a result slot it would
                  -- then be removed from, which would silently shorten a caller's `limit`.
                  AND NOT EXISTS (
                      SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM memory_supersession s
                      JOIN memory_versions newer ON newer.version_id = s.version_id
                      WHERE s.superseded_version_id = v.version_id
                        AND newer.valid_from_ns <= ?5
                  )
                  AND (?9 IS NULL OR v.version_id IN (SELECT value FROM json_each(?9)))
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
                  -- A withdrawn memory is gone as far as every reader is concerned. Filtered in
                  -- SQL rather than after the fact so it never occupies a result slot it would
                  -- then be removed from, which would silently shorten a caller's `limit`.
                  AND NOT EXISTS (
                      SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
                  )
                  AND NOT EXISTS (
                      SELECT 1
                      FROM memory_supersession s
                      JOIN memory_versions newer ON newer.version_id = s.version_id
                      WHERE s.superseded_version_id = v.version_id
                        AND newer.valid_from_ns <= ?5
                  )
                  AND (?9 IS NULL OR v.version_id IN (SELECT value FROM json_each(?9)))
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
                limit,
                restrict
            ],
            parse_memory_hit,
        )?;
        rows.map(|row| row?.into_hit(query.project_id)).collect()
    }
}

/// Build the FTS5 match expression for a query's text.
///
/// Terms are joined with `OR`, not `AND`, because the result set is ranked. `ORDER BY
/// bm25(...)` already places documents matching more of the query — and rarer parts of it —
/// above documents matching one common word, which is what BM25 is for. Requiring every term
/// instead turns the ranking off: nothing reaches it unless it already matched everything.
///
/// That distinction is invisible on the queries this was built for. `PREFS_ENABLED`, a commit
/// sha, an `event:` uuid — one or two rare terms, where `AND` and `OR` return the same rows.
/// It is decisive on the queries a memory system actually receives. Measured on LongMemEval-S,
/// "What degree did I graduate with?" under `AND` demands that one event contain *what*, *did*,
/// *I* and *with* together, and the benchmark scored **0.0% R@5** across every question type
/// before this changed.
fn match_expression(_project_id: ProjectId, text: Option<&str>) -> Option<String> {
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
    Some(terms.join(" OR "))
}

/// Fuse ranked channels by Reciprocal Rank Fusion.
///
/// RRF scores a document by where each channel *placed* it, never by what each channel *scored*
/// it. That is the whole point: a BM25 score and a cosine similarity are numbers on incompatible
/// scales, and any attempt to combine them directly needs a normalisation that is itself a
/// guess — one that shifts every time the corpus grows. Ranks need no such calibration, which is
/// why this stays correct as the ledger changes underneath it.
///
/// Fusing one channel is the identity. Its score, `weight / (k + rank + 1)`, falls strictly as
/// rank rises, so the input order comes out unchanged and no caller needs a special case for a
/// brain that has only keyword search.
fn fuse(channels: &[(f64, Vec<SearchHit>)]) -> Vec<SearchHit> {
    let mut scores: HashMap<uuid::Uuid, f64> = HashMap::new();
    let mut fused: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<uuid::Uuid> = HashSet::new();
    for (weight, hits) in channels {
        for (rank, hit) in hits.iter().enumerate() {
            *scores.entry(hit.source_id).or_default() += weight / (RRF_K + rank as f64 + 1.0);
            // The first channel to produce a document supplies its body. Channels are ordered
            // with keyword first, so a document found by both keeps its real BM25 score rather
            // than the 0.0 the vector channel's hydration carries.
            if seen.insert(hit.source_id) {
                fused.push(hit.clone());
            }
        }
    }
    for hit in &mut fused {
        hit.rank_score = scores.get(&hit.source_id).copied().unwrap_or(0.0);
    }
    fused.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.occurred_at.cmp(&left.occurred_at))
            .then_with(|| right.source_id.cmp(&left.source_id))
    });
    fused
}

/// Push a session's overflow behind the other sessions, preserving relative order within each.
///
/// Demotion, not removal. Ten near-identical turns from one long session is a worse answer than
/// the same budget spread across the sessions that differ — but a capped hit is still the best
/// answer available if nothing else matched, so it moves to the tail rather than out of the
/// result. That makes this reorder recall without ever costing any.
///
/// Memories carry no session, so they are never capped. That is correct rather than convenient:
/// a memory is already the distillation of a session, so limiting them by session would be
/// deduplicating something that was deduplicated when it was written.
fn diversify_by_session(hits: &mut Vec<SearchHit>) {
    let mut per_session: HashMap<&str, usize> = HashMap::new();
    let mut kept = Vec::with_capacity(hits.len());
    let mut demoted = Vec::new();
    for hit in hits.iter() {
        match hit.native_session_id.as_deref() {
            Some(session) => {
                let count = per_session.entry(session).or_default();
                *count += 1;
                if *count <= MAX_HITS_PER_SESSION {
                    kept.push(hit.clone());
                } else {
                    demoted.push(hit.clone());
                }
            }
            None => kept.push(hit.clone()),
        }
    }
    kept.append(&mut demoted);
    *hits = kept;
}

fn by_source_id(hits: Vec<SearchHit>) -> HashMap<uuid::Uuid, SearchHit> {
    hits.into_iter().map(|hit| (hit.source_id, hit)).collect()
}

/// Render ids as a JSON array, for `json_each` to expand inside a statement.
///
/// The alternative is building `?9, ?10, …` to match the id count, which makes the SQL text
/// depend on the data and defeats SQLite's statement cache — a new plan compiled for every
/// distinct number of candidates.
fn id_list(ids: &[uuid::Uuid]) -> String {
    let rendered: Vec<String> = ids.iter().map(uuid::Uuid::to_string).collect();
    serde_json::to_string(&rendered).unwrap_or_else(|_| "[]".to_owned())
}

fn estimated_cache_entry_bytes(query: &SearchQuery, hits: &[SearchHit]) -> usize {
    let query_bytes = query.text.as_ref().map_or(0, String::len)
        + query.native_session_id.as_ref().map_or(0, String::len)
        + 256;
    hits.iter().fold(query_bytes, |total, hit| {
        total.saturating_add(
            hit.title.len()
                + hit.text.len()
                + hit.path.as_ref().map_or(0, String::len)
                + hit.native_session_id.as_ref().map_or(0, String::len)
                + 256,
        )
    })
}

struct RawEventHit {
    source_id: String,
    worktree_id: String,
    task_id: Option<String>,
    native_session_id: String,
    harness: String,
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
        harness: row.get(4)?,
        event_type: row.get(5)?,
        occurred_at_ns: row.get(6)?,
        observed_at_ns: row.get(7)?,
        source_locator: row.get(8)?,
        payload_json: row.get(9)?,
        bm25_score: row.get(10)?,
    })
}

impl RawEventHit {
    fn into_hit(self, project_id: ProjectId) -> Result<SearchHit> {
        let occurred_at = from_ns(self.occurred_at_ns)?;
        let observed_at = from_ns(self.observed_at_ns)?;
        Ok(SearchHit {
            source: SearchSource::Event,
            source_id: uuid::Uuid::parse_str(&self.source_id)?,
            harness: Some(match self.harness.as_str() {
                "claude-code" => Harness::ClaudeCode,
                "codex" => Harness::Codex,
                "hermes" => Harness::Hermes,
                other => Harness::Other(other.to_owned()),
            }),
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
            rank_score: self.bm25_score,
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
            harness: None,
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
            rank_score: self.bm25_score,
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

#[cfg(test)]
mod tests {
    use brain_domain::{
        EventBatch, EventType, Harness, NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
    };

    use super::{SearchQuery, match_expression};
    use crate::EventLedger;

    #[test]
    fn fts_match_uses_only_selective_user_terms_not_the_ubiquitous_project_token() {
        let project = ProjectId(
            uuid::Uuid::parse_str("00000000-0000-7000-8000-000000000001")
                .expect("literal project UUID"),
        );

        assert_eq!(
            match_expression(project, Some("OAuth PKCE")),
            Some("\"OAuth\" OR \"PKCE\"".to_owned())
        );
    }

    #[test]
    fn a_natural_language_question_does_not_require_every_word_to_co_occur() {
        // The defect this guards was measured, not imagined: joined with AND, this question
        // demanded a single event containing "What", "did", "I" and "with" together, and
        // LongMemEval-S scored 0.0% R@5 across all six question types. Ranking is what
        // separates a good hit from a weak one; AND stops anything reaching the ranking.
        let project = ProjectId(uuid::Uuid::now_v7());
        let expression = match_expression(project, Some("What degree did I graduate with?"))
            .expect("a question produces terms");
        assert!(
            !expression.contains(" AND "),
            "terms must not be conjunctive, got {expression}"
        );
        assert!(expression.contains("\"degree\" OR "));
        assert!(expression.ends_with("\"with\""));
    }

    #[test]
    fn cached_search_is_invalidated_by_a_local_append() {
        let project = ProjectId(uuid::Uuid::now_v7());
        let worktree = WorktreeId(uuid::Uuid::now_v7());
        let mut ledger = EventLedger::open_in_memory(project).expect("ledger");
        let query = SearchQuery::text(project, "new sentinel").events_only();

        assert!(ledger.search(&query).expect("initial search").is_empty());
        assert_eq!(ledger.search_cache.borrow().entries.len(), 1);

        ledger
            .append_batch(&EventBatch {
                source_id: "cache-test".to_owned(),
                events: vec![cache_event(project, worktree, [2; 32])],
                quarantined: Vec::new(),
                capture_gaps: Vec::new(),
                next_cursor: SourceCursor::byte_offset(1),
            })
            .expect("append");

        let hits = ledger.search(&query).expect("search after append");
        assert_eq!(
            hits.len(),
            1,
            "append must invalidate an empty cached result"
        );
    }

    #[test]
    fn cached_search_is_invalidated_by_an_external_commit() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("ledger.sqlite");
        let project = ProjectId(uuid::Uuid::now_v7());
        let worktree = WorktreeId(uuid::Uuid::now_v7());
        let first = EventLedger::open(&path, project).expect("first ledger");
        let mut second = EventLedger::open(&path, project).expect("second ledger");
        let query = SearchQuery::text(project, "new sentinel").events_only();

        assert!(first.search(&query).expect("initial search").is_empty());
        second
            .append_batch(&EventBatch {
                source_id: "external-cache-test".to_owned(),
                events: vec![cache_event(project, worktree, [3; 32])],
                quarantined: Vec::new(),
                capture_gaps: Vec::new(),
                next_cursor: SourceCursor::byte_offset(1),
            })
            .expect("external append");

        assert_eq!(
            first
                .search(&query)
                .expect("search after external commit")
                .len(),
            1,
            "SQLite data_version must invalidate results cached before an external commit"
        );
    }

    #[test]
    fn search_cache_never_exceeds_its_entry_cap() {
        let project = ProjectId(uuid::Uuid::now_v7());
        let ledger = EventLedger::open_in_memory(project).expect("ledger");

        for index in 0..=super::MAX_SEARCH_CACHE_ENTRIES {
            ledger
                .search(
                    &SearchQuery::text(project, format!("absent-cache-key-{index}")).events_only(),
                )
                .expect("search");
        }

        let cache = ledger.search_cache.borrow();
        assert!(cache.entries.len() <= super::MAX_SEARCH_CACHE_ENTRIES);
        assert!(cache.estimated_bytes <= super::MAX_SEARCH_CACHE_BYTES);
    }

    #[test]
    fn fusing_one_channel_leaves_its_order_untouched() {
        // The property every brain without an embedding model depends on. RRF's score falls
        // strictly with rank, so a lone channel comes out exactly as it went in — which is what
        // lets the fused path be the only path, with no keyword-only branch to keep in step.
        let hits: Vec<_> = (0..25).map(|index| stub_hit(index, 0.0)).collect();
        let before: Vec<_> = hits.iter().map(|hit| hit.source_id).collect();

        let fused = super::fuse(&[(super::BM25_WEIGHT, hits)]);

        assert_eq!(
            fused.iter().map(|hit| hit.source_id).collect::<Vec<_>>(),
            before
        );
        assert!(
            fused
                .windows(2)
                .all(|pair| pair[0].rank_score > pair[1].rank_score),
            "a single channel's fused scores must be strictly decreasing"
        );
    }

    #[test]
    fn a_document_only_the_vector_channel_found_can_outrank_a_weak_keyword_match() {
        // The point of fusing at all. A document no keyword matched has no BM25 score to carry
        // it, so if fusion did not lift it by its vector rank the channel would be decoration.
        let keyword: Vec<_> = (0..20).map(|index| stub_hit(index, 5.0)).collect();
        let only_semantic = stub_hit(99, 0.0);
        let vector = vec![only_semantic.clone()];

        let fused = super::fuse(&[
            (super::BM25_WEIGHT, keyword),
            (super::VECTOR_WEIGHT, vector),
        ]);

        let rank = fused
            .iter()
            .position(|hit| hit.source_id == only_semantic.source_id)
            .expect("the vector-only hit survives fusion");
        assert_eq!(
            rank, 0,
            "top of one channel with the heavier weight must lead, got rank {rank}"
        );
    }

    #[test]
    fn a_document_both_channels_found_keeps_its_real_bm25_score() {
        // The vector channel hydrates its results without a MATCH, so its copies carry 0.0. If
        // fusion took the later copy, every re-ranking caller downstream would read a zero for a
        // document that keyword search scored well.
        let shared = stub_hit(7, 4.25);
        let mut zeroed = shared.clone();
        zeroed.bm25_score = 0.0;

        let fused = super::fuse(&[
            (super::BM25_WEIGHT, vec![shared.clone()]),
            (super::VECTOR_WEIGHT, vec![zeroed]),
        ]);

        assert_eq!(fused.len(), 1, "one document, not two");
        assert_eq!(fused[0].bm25_score, 4.25);
        assert!(
            fused[0].rank_score > super::BM25_WEIGHT / (super::RRF_K + 1.0),
            "a document both channels found must score above either channel alone"
        );
    }

    #[test]
    fn session_diversification_demotes_overflow_instead_of_dropping_it() {
        // Recall may be reordered here; it may never be lost. A capped hit is still the best
        // answer available if nothing else matched.
        let mut hits: Vec<_> = (0..6)
            .map(|index| {
                let mut hit = stub_hit(index, 1.0);
                hit.native_session_id = Some("one-long-session".to_owned());
                hit
            })
            .collect();
        let mut other = stub_hit(50, 0.5);
        other.native_session_id = Some("another-session".to_owned());
        hits.push(other.clone());
        let before = hits.len();

        super::diversify_by_session(&mut hits);

        assert_eq!(hits.len(), before, "diversification must not drop hits");
        assert_eq!(
            hits[super::MAX_HITS_PER_SESSION].source_id,
            other.source_id,
            "the second session must be promoted past the first's overflow"
        );
        assert!(
            hits[super::MAX_HITS_PER_SESSION + 1..]
                .iter()
                .all(|hit| hit.native_session_id.as_deref() == Some("one-long-session")),
            "overflow belongs behind the other sessions, not gone"
        );
    }

    #[test]
    fn memories_are_never_capped_by_session() {
        // A memory carries no session because it is already the distillation of one. Capping
        // them by session would deduplicate something deduplicated when it was written.
        let mut hits: Vec<_> = (0..6)
            .map(|index| {
                let mut hit = stub_hit(index, 1.0);
                hit.source = super::SearchSource::Memory;
                hit.native_session_id = None;
                hit
            })
            .collect();
        let before: Vec<_> = hits.iter().map(|hit| hit.source_id).collect();

        super::diversify_by_session(&mut hits);

        assert_eq!(
            hits.iter().map(|hit| hit.source_id).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn an_empty_id_restriction_returns_nothing_rather_than_everything() {
        // `json_each('[]')` yields no rows, so the SQL is right either way — but the guard makes
        // it impossible for a future edit that drops the clause to turn "no candidates" into
        // "the whole project", which is the direction this mistake always fails in.
        let project = ProjectId(uuid::Uuid::now_v7());
        let worktree = WorktreeId(uuid::Uuid::now_v7());
        let mut ledger = EventLedger::open_in_memory(project).expect("ledger");
        ledger
            .append_batch(&EventBatch {
                source_id: "restriction".to_owned(),
                events: vec![cache_event(project, worktree, [9; 32])],
                quarantined: Vec::new(),
                capture_gaps: Vec::new(),
                next_cursor: SourceCursor::byte_offset(1),
            })
            .expect("append");
        let query = SearchQuery::text(project, "new sentinel").events_only();

        assert_eq!(
            ledger.search_events(&query, 10, None).expect("open").len(),
            1
        );
        assert!(
            ledger
                .search_events(&query, 10, Some(&[]))
                .expect("empty restriction")
                .is_empty()
        );
    }

    fn stub_hit(seed: u8, bm25_score: f64) -> super::SearchHit {
        super::SearchHit {
            source: super::SearchSource::Event,
            source_id: uuid::Uuid::from_bytes([seed; 16]),
            harness: Some(Harness::ClaudeCode),
            memory_id: None,
            project_id: ProjectId(uuid::Uuid::nil()),
            worktree_id: None,
            task_id: None,
            native_session_id: Some(format!("session-{seed}")),
            kind: None,
            title: "stub".to_owned(),
            text: "stub".to_owned(),
            path: None,
            occurred_at: time::OffsetDateTime::UNIX_EPOCH,
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            late_observation: false,
            authority: None,
            status: None,
            evidence_count: 0,
            bm25_score,
            rank_score: bm25_score,
        }
    }

    fn cache_event(
        project_id: ProjectId,
        worktree_id: WorktreeId,
        idempotency_key: [u8; 32],
    ) -> NormalizedEvent {
        NormalizedEvent {
            event_id: uuid::Uuid::now_v7(),
            project_id,
            worktree_id,
            task_id: None,
            harness: Harness::Codex,
            native_session_id: "cache-session".to_owned(),
            native_turn_id: None,
            event_type: EventType::AgentResponded,
            occurred_at: time::OffsetDateTime::UNIX_EPOCH,
            observed_at: time::OffsetDateTime::UNIX_EPOCH,
            source_locator: "cache.jsonl".to_owned(),
            source_offset: 1,
            source_schema: "cache:v1".to_owned(),
            raw_hash: [1; 32],
            idempotency_key,
            git_head: None,
            git_branch: Some("main".to_owned()),
            payload: serde_json::json!({"message": "new sentinel"}),
            raw: serde_json::json!({"message": "new sentinel"}),
        }
    }
}
