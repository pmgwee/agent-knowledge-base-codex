use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};

use crate::EventLedger;
use crate::cursor::timestamp_ns;

impl EventLedger {
    pub fn note_import_hash(&self, note_path: &str) -> Result<Option<[u8; 32]>> {
        let bytes = self
            .connection
            .query_row(
                "SELECT content_hash FROM note_imports WHERE project_id = ?1 AND note_path = ?2",
                params![self.project_scope.0.to_string(), note_path],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("stored note hash is not 32 bytes"))
            })
            .transpose()
    }

    pub fn record_note_import(
        &mut self,
        note_path: &str,
        content_hash: [u8; 32],
        memory_id: uuid::Uuid,
        version_id: uuid::Uuid,
        imported_at: time::OffsetDateTime,
    ) -> Result<()> {
        ensure!(!note_path.is_empty(), "note path cannot be empty");
        let transaction = self.connection.transaction()?;
        transaction.execute(
            r#"
            INSERT INTO note_imports(
                project_id, note_path, content_hash, memory_id, version_id, imported_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(project_id, note_path) DO UPDATE SET
                content_hash = excluded.content_hash,
                memory_id = excluded.memory_id,
                version_id = excluded.version_id,
                imported_at_ns = excluded.imported_at_ns
            "#,
            params![
                self.project_scope.0.to_string(),
                note_path,
                content_hash.as_slice(),
                memory_id.to_string(),
                version_id.to_string(),
                timestamp_ns(imported_at)?,
            ],
        )?;
        transaction.execute(
            r#"
            UPDATE note_review_queue
            SET resolved_at_ns = ?3
            WHERE project_id = ?1 AND note_path = ?2 AND resolved_at_ns IS NULL
            "#,
            params![
                self.project_scope.0.to_string(),
                note_path,
                timestamp_ns(imported_at)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn queue_note_review(
        &mut self,
        note_path: &str,
        content_hash: [u8; 32],
        reason: &str,
        observed_at: time::OffsetDateTime,
    ) -> Result<bool> {
        ensure!(!note_path.is_empty(), "note path cannot be empty");
        ensure!(!reason.is_empty(), "note review reason cannot be empty");
        Ok(self.connection.execute(
            r#"
            INSERT OR IGNORE INTO note_review_queue(
                project_id, note_path, content_hash, reason, observed_at_ns, resolved_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
            params![
                self.project_scope.0.to_string(),
                note_path,
                content_hash.as_slice(),
                reason.chars().take(500).collect::<String>(),
                timestamp_ns(observed_at)?,
            ],
        )? > 0)
    }

    pub fn note_review_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM note_review_queue WHERE project_id = ?1 AND resolved_at_ns IS NULL",
            [self.project_scope.0.to_string()],
            |row| row.get(0),
        )?)
    }

    pub fn evidence_belongs_to_project(&self, evidence_ids: &[uuid::Uuid]) -> Result<bool> {
        for evidence_id in evidence_ids {
            let exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE project_id = ?1 AND event_id = ?2)",
                params![self.project_scope.0.to_string(), evidence_id.to_string()],
                |row| row.get(0),
            )?;
            if !exists {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn imported_note_version(&self, note_path: &str) -> Result<Option<uuid::Uuid>> {
        let value = self
            .connection
            .query_row(
                "SELECT version_id FROM note_imports WHERE project_id = ?1 AND note_path = ?2",
                params![self.project_scope.0.to_string(), note_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        value
            .map(|value| uuid::Uuid::parse_str(&value).context("stored note version is invalid"))
            .transpose()
    }
}
