//! Storing a subject page's paragraph against the memory set it describes.
//!
//! Keyed by a hash of the memory set, not by the subject's name. Prose that correctly describes
//! five memories is simply wrong once there are seven, and a subject gains memories continuously —
//! that is the whole point of a subject page. Keying on the set means a changed subject has **no**
//! synthesis rather than a stale one, and the page falls back to listing links, which was already
//! known to be safe.
//!
//! The alternative, keying by term and regenerating on a schedule, has a window in which the page
//! asserts something the ledger no longer supports. There is no length of window that is fine.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use sha2::Digest;

use crate::cursor::timestamp_ns;
use crate::ledger::EventLedger;

/// A stored paragraph and the set it was written about.
#[derive(Clone, Debug)]
pub struct StoredSynthesis {
    pub term: String,
    pub markdown: String,
    pub generated_at: time::OffsetDateTime,
}

/// Fingerprint a subject's memory set.
///
/// Sorted before hashing, so the same set in a different order is the same set. Retrieval order is
/// not part of what the prose describes, and letting it be would discard perfectly good synthesis
/// every time ranking shifted.
pub fn memory_set_hash(memory_ids: &[uuid::Uuid]) -> String {
    let mut sorted: Vec<uuid::Uuid> = memory_ids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut hasher = sha2::Sha256::new();
    for id in sorted {
        hasher.update(id.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

impl EventLedger {
    /// Store a validated paragraph for a subject, replacing any earlier one.
    ///
    /// Takes the markdown rather than the proposal: validation belongs to `brain-context`, and a
    /// store that accepted an unvalidated proposal would make the validator optional.
    pub fn store_subject_synthesis(
        &self,
        term: &str,
        memory_ids: &[uuid::Uuid],
        markdown: &str,
        now: time::OffsetDateTime,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO subject_synthesis(
                 project_id, term, memory_set_hash, markdown, generated_at_ns
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id, term) DO UPDATE SET
                 memory_set_hash = excluded.memory_set_hash,
                 markdown = excluded.markdown,
                 generated_at_ns = excluded.generated_at_ns",
            params![
                self.project_scope.0.to_string(),
                term,
                memory_set_hash(memory_ids),
                markdown,
                timestamp_ns(now)?,
            ],
        )?;
        Ok(())
    }

    /// The stored paragraph for a subject, **only if it still describes this exact memory set**.
    ///
    /// Returning `None` on a mismatch rather than the stale text is the whole design: a subject page
    /// with no paragraph is a page that asserts nothing, which cannot be wrong.
    pub fn subject_synthesis(
        &self,
        term: &str,
        memory_ids: &[uuid::Uuid],
    ) -> Result<Option<StoredSynthesis>> {
        self.connection
            .query_row(
                "SELECT markdown, generated_at_ns FROM subject_synthesis
                 WHERE project_id = ?1 AND term = ?2 AND memory_set_hash = ?3",
                params![
                    self.project_scope.0.to_string(),
                    term,
                    memory_set_hash(memory_ids),
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .map(|(markdown, at)| {
                Ok(StoredSynthesis {
                    term: term.to_owned(),
                    markdown,
                    generated_at: time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(at))?,
                })
            })
            .transpose()
    }

    /// How many subjects currently carry a paragraph.
    pub fn subject_synthesis_count(&self) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM subject_synthesis WHERE project_id = ?1",
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_set_in_a_different_order_hashes_the_same() {
        // Retrieval order is not part of what the prose describes. Letting it matter would discard
        // good synthesis every time ranking shifted.
        let a = uuid::Uuid::now_v7();
        let b = uuid::Uuid::now_v7();
        assert_eq!(memory_set_hash(&[a, b]), memory_set_hash(&[b, a]));
        assert_eq!(memory_set_hash(&[a, b, a]), memory_set_hash(&[b, a]));
    }

    #[test]
    fn a_different_set_hashes_differently() {
        let a = uuid::Uuid::now_v7();
        let b = uuid::Uuid::now_v7();
        assert_ne!(memory_set_hash(&[a]), memory_set_hash(&[a, b]));
    }
}
