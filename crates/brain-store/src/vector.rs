//! Storing and searching dense vectors for memories.
//!
//! The index is exhaustive: every stored vector is compared against the query. That is the right
//! shape at this size — a few thousand memories per project, 384 floats each, is under ten
//! megabytes and scans in milliseconds — and it has the property an approximate index does not,
//! which is that recall is exactly what the model gives it. An ANN structure trades some of that
//! away for a speed nobody here needs yet. When a project reaches a scale where scanning hurts,
//! the fix is a real index; guessing at one now would be optimising a cost that has not appeared.
//!
//! Both layers are embedded — memories and raw events — for different reasons.
//!
//! Memories are the distilled layer, so a question about what someone decided should match a
//! decision rather than the turn it was mentioned in, and at eleven a second the ~8,500 memories
//! here take about ten minutes.
//!
//! Events are slower (~135,000 of them, a few hours once) and were initially left out on that
//! basis. That was wrong, and measurably so: the one benchmark category this work exists to fix,
//! `single-session-preference` at 63.3%, is answered by a raw user turn, not by a memory. So is
//! "what did I do last week". Embedding only the distilled layer would have produced a vector
//! index that could not reach the evidence it was built for — and on the benchmark, whose ledgers
//! hold events and no memories at all, it would have changed exactly nothing while appearing to
//! be a new retrieval channel.

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};

use crate::embedding::{EMBEDDING_DIMENSIONS, cosine_similarity, decode_vector, encode_vector};
use crate::ledger::EventLedger;

/// Identifier recorded alongside every vector.
///
/// Vectors from different models are not comparable, so this is what lets a model change be a
/// filter rather than a silent corruption of every score.
pub const EMBEDDING_MODEL: &str = "all-MiniLM-L6-v2";

/// A memory version awaiting an embedding, with the text to embed.
#[derive(Clone, Debug)]
pub struct PendingEmbedding {
    pub version_id: uuid::Uuid,
    pub memory_id: uuid::Uuid,
    /// Title and content joined. Both are embedded because a title carries the claim and the
    /// content carries the detail, and a query may resemble either.
    pub text: String,
}

/// One vector search result.
#[derive(Clone, Debug)]
pub struct VectorHit {
    pub memory_id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub similarity: f32,
}

/// An event awaiting an embedding, with the text to embed.
#[derive(Clone, Debug)]
pub struct PendingEventEmbedding {
    pub event_id: uuid::Uuid,
    pub text: String,
}

/// One event vector search result.
#[derive(Clone, Debug)]
pub struct EventVectorHit {
    pub event_id: uuid::Uuid,
    pub similarity: f32,
}

/// Shortest event text worth embedding.
///
/// Most of what a harness records is not prose: a tool result of `{}`, a one-word status, a bare
/// path. Embedding those spends the expensive part of the pipeline on strings that carry no
/// meaning to match against, and puts vectors in the index whose nearest neighbour is noise.
const MIN_EMBEDDABLE_CHARACTERS: usize = 24;

impl EventLedger {
    /// Current memory versions that have no vector for the active model.
    ///
    /// Only current versions: embedding a superseded one spends time to make a memory findable
    /// by wording it no longer has. `limit` bounds a pass so a backfill yields to other work
    /// rather than holding the ledger for its whole duration.
    pub fn memories_awaiting_embedding(&self, limit: usize) -> Result<Vec<PendingEmbedding>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT v.version_id, v.memory_id, v.title, v.content
            FROM memory_versions v
            JOIN memory_records r ON r.memory_id = v.memory_id
            LEFT JOIN memory_embeddings e
                   ON e.version_id = v.version_id AND e.model = ?2
            WHERE r.project_id = ?1
              AND e.version_id IS NULL
              AND v.status = 'current'
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
            ORDER BY v.recorded_at_ns DESC
            LIMIT ?3
            "#,
        )?;
        let rows = statement.query_map(
            params![
                self.project_scope.0.to_string(),
                EMBEDDING_MODEL,
                i64::try_from(limit)?
            ],
            |row| -> rusqlite::Result<(String, String, String, String)> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            },
        )?;
        rows.map(|row| {
            let (version_id, memory_id, title, content) = row?;
            Ok(PendingEmbedding {
                version_id: uuid::Uuid::parse_str(&version_id)?,
                memory_id: uuid::Uuid::parse_str(&memory_id)?,
                text: format!("{title}\n\n{content}"),
            })
        })
        .collect()
    }

    /// Record a vector, replacing any earlier one for the same version and model.
    pub fn store_memory_embedding(
        &mut self,
        pending: &PendingEmbedding,
        vector: &[f32],
        now: time::OffsetDateTime,
    ) -> Result<()> {
        anyhow::ensure!(
            vector.len() == EMBEDDING_DIMENSIONS,
            "refusing to store a {}-dimensional vector; this ledger stores {EMBEDDING_DIMENSIONS}",
            vector.len()
        );
        self.connection
            .execute(
                r#"
                INSERT INTO memory_embeddings(
                    version_id, memory_id, project_id, model, dimensions, vector, embedded_at_ns
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(version_id) DO UPDATE SET
                    model = excluded.model,
                    dimensions = excluded.dimensions,
                    vector = excluded.vector,
                    embedded_at_ns = excluded.embedded_at_ns
                "#,
                params![
                    pending.version_id.to_string(),
                    pending.memory_id.to_string(),
                    self.project_scope.0.to_string(),
                    EMBEDDING_MODEL,
                    i64::try_from(EMBEDDING_DIMENSIONS)?,
                    encode_vector(vector),
                    now.unix_timestamp_nanos() as i64,
                ],
            )
            .context("store memory embedding")?;
        Ok(())
    }

    /// How many current memory versions have a vector, and how many do not.
    pub fn embedding_coverage(&self) -> Result<(u64, u64)> {
        let embedded: i64 = self
            .connection
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM memory_embeddings e
                JOIN memory_versions v ON v.version_id = e.version_id
                WHERE e.project_id = ?1 AND e.model = ?2 AND v.status = 'current'
                "#,
                params![self.project_scope.0.to_string(), EMBEDDING_MODEL],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let total: i64 = self
            .connection
            .query_row(
                r#"
                SELECT COUNT(*)
                FROM memory_versions v
                JOIN memory_records r ON r.memory_id = v.memory_id
                WHERE r.project_id = ?1 AND v.status = 'current'
                "#,
                params![self.project_scope.0.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok((
            u64::try_from(embedded).unwrap_or(0),
            u64::try_from(total.saturating_sub(embedded)).unwrap_or(0),
        ))
    }

    /// The `limit` memories most similar to `query_vector`, best first.
    ///
    /// Rows whose stored width does not match are skipped rather than scored: that means a
    /// different model wrote them, and comparing coordinates that do not correspond would
    /// produce a number that looks like a similarity and means nothing.
    pub fn search_by_vector(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>> {
        if limit == 0 || query_vector.len() != EMBEDDING_DIMENSIONS {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            r#"
            SELECT e.memory_id, e.version_id, e.vector
            FROM memory_embeddings e
            JOIN memory_versions v ON v.version_id = e.version_id
            WHERE e.project_id = ?1 AND e.model = ?2 AND v.status = 'current'
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_scope.0.to_string(), EMBEDDING_MODEL],
            |row| -> rusqlite::Result<(String, String, Vec<u8>)> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            },
        )?;

        let mut hits = Vec::new();
        for row in rows {
            let (memory_id, version_id, blob) = row?;
            let Some(vector) = decode_vector(&blob) else {
                continue;
            };
            hits.push(VectorHit {
                memory_id: uuid::Uuid::parse_str(&memory_id)?,
                version_id: uuid::Uuid::parse_str(&version_id)?,
                similarity: cosine_similarity(query_vector, &vector),
            });
        }
        // Descending similarity, ties broken by id so the ordering is stable across runs and a
        // fused ranking downstream cannot shift for reasons nothing observed.
        hits.sort_by(|left, right| {
            right
                .similarity
                .total_cmp(&left.similarity)
                .then_with(|| left.memory_id.cmp(&right.memory_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// Every current memory's vector, keyed by memory id.
    ///
    /// For deriving structure over the memory set rather than answering a query — subject pages
    /// need to know which memories cluster, which is a property of the whole set and not of any
    /// one search.
    pub fn current_memory_vectors(&self) -> Result<Vec<(uuid::Uuid, Vec<f32>)>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT e.memory_id, e.vector
            FROM memory_embeddings e
            JOIN memory_versions v ON v.version_id = e.version_id
            WHERE e.project_id = ?1 AND e.model = ?2 AND v.status = 'current'
              AND NOT EXISTS (
                  SELECT 1 FROM memory_tombstones t WHERE t.memory_id = v.memory_id
              )
            ORDER BY e.memory_id
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_scope.0.to_string(), EMBEDDING_MODEL],
            |row| -> rusqlite::Result<(String, Vec<u8>)> { Ok((row.get(0)?, row.get(1)?)) },
        )?;
        let mut vectors = Vec::new();
        for row in rows {
            let (memory_id, blob) = row?;
            let Some(vector) = decode_vector(&blob) else {
                continue;
            };
            vectors.push((uuid::Uuid::parse_str(&memory_id)?, vector));
        }
        Ok(vectors)
    }

    /// Events with no vector for the active model, newest first.
    ///
    /// Newest first because a backfill that starts at the oldest turn makes the index useful last;
    /// starting at the newest means recent work — what orientation and "what did I do last week"
    /// both ask about — is searchable within minutes rather than hours.
    ///
    /// The text is the same text FTS indexes, so the two channels agree about what a document
    /// says. Events too short to carry meaning are filtered here rather than embedded and ignored
    /// later, because an excluded row is offered again on every pass otherwise.
    pub fn events_awaiting_embedding(&self, limit: usize) -> Result<Vec<PendingEventEmbedding>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT e.event_id,
                   coalesce(
                       nullif(c.search_text, ''),
                       json_extract(e.payload_json, '$.content'),
                       json_extract(e.payload_json, '$.text'),
                       json_extract(e.payload_json, '$.summary'),
                       json_extract(e.payload_json, '$.message'),
                       e.payload_json
                   )
            FROM events e
            LEFT JOIN event_segment_catalog c ON c.event_id = e.event_id
            LEFT JOIN event_embeddings x
                   ON x.event_id = e.event_id AND x.model = ?2
            WHERE e.project_id = ?1
              AND x.event_id IS NULL
            ORDER BY e.occurred_at_ns DESC
            LIMIT ?3
            "#,
        )?;
        let rows = statement.query_map(
            params![
                self.project_scope.0.to_string(),
                EMBEDDING_MODEL,
                i64::try_from(limit)?
            ],
            |row| -> rusqlite::Result<(String, Option<String>)> { Ok((row.get(0)?, row.get(1)?)) },
        )?;
        let mut pending = Vec::new();
        for row in rows {
            let (event_id, text) = row?;
            let text = text.unwrap_or_default();
            pending.push(PendingEventEmbedding {
                event_id: uuid::Uuid::parse_str(&event_id)?,
                text,
            });
        }
        Ok(pending)
    }

    /// Whether an event's text is worth a vector.
    ///
    /// Public so a backfill can record the skipped ones as embedded-with-no-vector rather than
    /// re-offering them forever; the decision belongs next to the query that produces them.
    pub fn is_embeddable_event_text(text: &str) -> bool {
        text.trim().chars().count() >= MIN_EMBEDDABLE_CHARACTERS
    }

    /// Record an event's vector. `None` marks the event considered and deliberately skipped.
    ///
    /// Skipped events are recorded with an empty vector rather than left absent, so the backfill's
    /// "what is left" query shrinks monotonically instead of stalling on rows it will never take.
    /// An empty vector is never returned by search, since decoding yields nothing to compare.
    pub fn store_event_embedding(
        &mut self,
        event_id: uuid::Uuid,
        vector: Option<&[f32]>,
        now: time::OffsetDateTime,
    ) -> Result<()> {
        if let Some(vector) = vector {
            anyhow::ensure!(
                vector.len() == EMBEDDING_DIMENSIONS,
                "refusing to store a {}-dimensional vector; this ledger stores {EMBEDDING_DIMENSIONS}",
                vector.len()
            );
        }
        let dimensions = vector.map_or(0, <[f32]>::len);
        self.connection
            .execute(
                r#"
                INSERT INTO event_embeddings(
                    event_id, project_id, model, dimensions, vector, embedded_at_ns
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(event_id) DO UPDATE SET
                    model = excluded.model,
                    dimensions = excluded.dimensions,
                    vector = excluded.vector,
                    embedded_at_ns = excluded.embedded_at_ns
                "#,
                params![
                    event_id.to_string(),
                    self.project_scope.0.to_string(),
                    EMBEDDING_MODEL,
                    i64::try_from(dimensions)?,
                    encode_vector(vector.unwrap_or(&[])),
                    now.unix_timestamp_nanos() as i64,
                ],
            )
            .context("store event embedding")?;
        Ok(())
    }

    /// How many events carry a usable vector, and how many are still unconsidered.
    ///
    /// Deliberately skipped events count as neither: they are done, but they are not searchable,
    /// and folding them into either number would misreport coverage in one direction or progress
    /// in the other.
    pub fn event_embedding_coverage(&self) -> Result<(u64, u64)> {
        let embedded: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM event_embeddings \
                 WHERE project_id = ?1 AND model = ?2 AND dimensions > 0",
                params![self.project_scope.0.to_string(), EMBEDDING_MODEL],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let remaining: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM events e \
                 LEFT JOIN event_embeddings x ON x.event_id = e.event_id AND x.model = ?2 \
                 WHERE e.project_id = ?1 AND x.event_id IS NULL",
                params![self.project_scope.0.to_string(), EMBEDDING_MODEL],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok((
            u64::try_from(embedded).unwrap_or(0),
            u64::try_from(remaining).unwrap_or(0),
        ))
    }

    /// The `limit` events most similar to `query_vector`, best first.
    pub fn search_events_by_vector(
        &self,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<EventVectorHit>> {
        if limit == 0 || query_vector.len() != EMBEDDING_DIMENSIONS {
            return Ok(Vec::new());
        }
        let mut statement = self.connection.prepare(
            "SELECT event_id, vector FROM event_embeddings \
             WHERE project_id = ?1 AND model = ?2 AND dimensions = ?3",
        )?;
        let rows = statement.query_map(
            params![
                self.project_scope.0.to_string(),
                EMBEDDING_MODEL,
                i64::try_from(EMBEDDING_DIMENSIONS)?
            ],
            |row| -> rusqlite::Result<(String, Vec<u8>)> { Ok((row.get(0)?, row.get(1)?)) },
        )?;

        let mut hits = Vec::new();
        for row in rows {
            let (event_id, blob) = row?;
            let Some(vector) = decode_vector(&blob) else {
                continue;
            };
            hits.push(EventVectorHit {
                event_id: uuid::Uuid::parse_str(&event_id)?,
                similarity: cosine_similarity(query_vector, &vector),
            });
        }
        hits.sort_by(|left, right| {
            right
                .similarity
                .total_cmp(&left.similarity)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}
