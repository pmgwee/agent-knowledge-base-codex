//! Storing and searching dense vectors for memories.
//!
//! The index is exhaustive: every stored vector is compared against the query. That is the right
//! shape at this size — a few thousand memories per project, 384 floats each, is under ten
//! megabytes and scans in milliseconds — and it has the property an approximate index does not,
//! which is that recall is exactly what the model gives it. An ANN structure trades some of that
//! away for a speed nobody here needs yet. When a project reaches a scale where scanning hurts,
//! the fix is a real index; guessing at one now would be optimising a cost that has not appeared.
//!
//! Only memories are embedded, not raw events. Memories are the distilled layer, so a question
//! about what someone decided should match a decision rather than the turn it was mentioned in;
//! and at eleven embeddings a second, the ~6,800 memories here take ten minutes while the
//! ~125,000 events would take five hours for a worse match.

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
}
