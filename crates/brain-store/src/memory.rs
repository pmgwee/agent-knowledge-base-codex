use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use brain_domain::{
    Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId, WorktreeId,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::EventLedger;
use crate::cursor::timestamp_ns;
use crate::migrations::{configure, migrate};

impl EventLedger {
    pub fn append_memory(&mut self, memory: &MemoryRecord) -> Result<()> {
        validate_record(memory)?;
        let MemoryScope::Project(project_id) = memory.scope else {
            bail!("project ledger cannot store global preferences");
        };
        ensure!(
            project_id == self.project_scope,
            "memory violates project scope {}",
            self.project_scope.0
        );
        let replay: Option<(String, String, String, String)> = self
            .connection
            .query_row(
                r#"
                SELECT v.memory_id, v.title, v.content, r.project_id
                FROM memory_versions v
                JOIN memory_records r ON r.memory_id = v.memory_id
                WHERE v.version_id = ?1
                "#,
                [memory.version_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some((memory_id, title, content, existing_project)) = replay {
            ensure!(
                memory_id == memory.id.to_string()
                    && title == memory.title
                    && content == memory.content
                    && existing_project == project_id.0.to_string(),
                "memory version ID collision has different content or scope"
            );
            return Ok(());
        }
        ensure!(
            !memory.evidence_ids.is_empty(),
            "project memory requires at least one evidence citation"
        );

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for evidence_id in &memory.evidence_ids {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE event_id = ?1 AND project_id = ?2)",
                params![evidence_id.to_string(), project_id.0.to_string()],
                |row| row.get(0),
            )?;
            ensure!(
                exists,
                "evidence {} does not belong to project {}",
                evidence_id,
                project_id.0
            );
        }

        let existing: Option<(String, String)> = transaction
            .query_row(
                "SELECT project_id, kind FROM memory_records WHERE memory_id = ?1",
                [memory.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((existing_project, existing_kind)) = existing {
            ensure!(
                existing_project == project_id.0.to_string()
                    && existing_kind == memory.kind.as_str(),
                "memory identity cannot change project scope or kind"
            );
        } else {
            transaction.execute(
                r#"
                INSERT INTO memory_records(
                    memory_id, scope, project_id, kind, projection_path, created_at_ns
                ) VALUES (?1, 'project', ?2, ?3, ?4, ?5)
                "#,
                params![
                    memory.id.to_string(),
                    project_id.0.to_string(),
                    memory.kind.as_str(),
                    memory.projection_path().to_string_lossy(),
                    timestamp_ns(memory.recorded_at)?,
                ],
            )?;
        }
        let version_number: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(version_number), 0) + 1 FROM memory_versions WHERE memory_id = ?1",
            [memory.id.to_string()],
            |row| row.get(0),
        )?;
        for superseded in &memory.supersedes {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_versions WHERE version_id = ?1)",
                [superseded.to_string()],
                |row| row.get(0),
            )?;
            ensure!(
                exists,
                "superseded memory version {superseded} does not exist"
            );
        }
        transaction.execute(
            r#"
            INSERT INTO memory_versions(
                version_id, memory_id, version_number, worktree_id, task_id,
                title, content, valid_from_ns, valid_to_ns, recorded_at_ns,
                confidence, authority, status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                memory.version_id.to_string(),
                memory.id.to_string(),
                version_number,
                memory.worktree_id.map(|id| id.0.to_string()),
                memory.task_id.map(|id| id.to_string()),
                memory.title,
                memory.content,
                timestamp_ns(memory.valid_from)?,
                memory.valid_to.map(timestamp_ns).transpose()?,
                timestamp_ns(memory.recorded_at)?,
                f64::from(memory.confidence),
                memory.authority.as_str(),
                memory.status.as_str(),
            ],
        )?;
        for evidence_id in &memory.evidence_ids {
            transaction.execute(
                "INSERT INTO memory_evidence(version_id, event_id) VALUES (?1, ?2)",
                params![memory.version_id.to_string(), evidence_id.to_string()],
            )?;
        }
        for superseded in &memory.supersedes {
            transaction.execute(
                "INSERT INTO memory_supersession(version_id, superseded_version_id) VALUES (?1, ?2)",
                params![memory.version_id.to_string(), superseded.to_string()],
            )?;
        }
        transaction.commit()?;
        // `PRAGMA data_version` does not move for a write on this same connection, so the search
        // cache cannot notice this and has to be told. `append_batch` already did; this did not,
        // which meant a process that wrote a memory and then searched could be served a result
        // predating it.
        self.search_cache.borrow_mut().clear();
        Ok(())
    }

    pub fn memory_versions(&self, memory_id: uuid::Uuid) -> Result<Vec<MemoryRecord>> {
        load_project_versions(&self.connection, self.project_scope, memory_id)
    }

    /// The current version of a memory, or `None` if it was withdrawn.
    ///
    /// Withdrawal is checked here rather than by callers so that a single lookup and a listing
    /// cannot disagree about whether a memory exists.
    pub fn current_memory(&self, memory_id: uuid::Uuid) -> Result<Option<MemoryRecord>> {
        if self.tombstone(memory_id)?.is_some() {
            return Ok(None);
        }
        Ok(self.memory_versions(memory_id)?.pop())
    }

    pub fn memory_version(&self, version_id: uuid::Uuid) -> Result<Option<MemoryRecord>> {
        let memory_id = self
            .connection
            .query_row(
                r#"
                SELECT v.memory_id
                FROM memory_versions v
                JOIN memory_records r ON r.memory_id = v.memory_id
                WHERE v.version_id = ?1 AND r.project_id = ?2
                "#,
                params![version_id.to_string(), self.project_scope.0.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|id| uuid::Uuid::parse_str(&id))
            .transpose()?;
        let Some(memory_id) = memory_id else {
            return Ok(None);
        };
        Ok(self
            .memory_versions(memory_id)?
            .into_iter()
            .find(|memory| memory.version_id == version_id))
    }

    pub fn memory_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))?)
    }

    pub fn memory_version_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM memory_versions", [], |row| row.get(0))?)
    }

    pub fn current_project_memories(&self) -> Result<Vec<MemoryRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT memory_id FROM memory_records WHERE project_id = ?1 ORDER BY projection_path",
        )?;
        let rows = statement.query_map([self.project_scope.0.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        let ids = rows
            .map(|row| Ok(uuid::Uuid::parse_str(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        // Withdrawn memories are filtered here rather than at each caller, because this is the
        // one query the projection and the export both go through. A read path that forgot would
        // resurrect a memory the user believed they had removed.
        let withdrawn = self.tombstoned_ids()?;
        let mut memories = Vec::with_capacity(ids.len());
        for id in ids {
            if withdrawn.contains(&id) {
                continue;
            }
            if let Some(memory) = self.current_memory(id)?
                && !matches!(
                    memory.status,
                    MemoryStatus::Invalid | MemoryStatus::Superseded
                )
            {
                memories.push(memory);
            }
        }
        memories.sort_by_key(MemoryRecord::projection_path);
        Ok(memories)
    }
}

pub struct GlobalPreferenceStore {
    connection: Connection,
}

impl GlobalPreferenceStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        configure(&connection)?;
        migrate(&connection)?;
        Ok(Self { connection })
    }

    pub fn append_preference(&mut self, memory: &MemoryRecord) -> Result<()> {
        validate_record(memory)?;
        ensure!(
            memory.scope == MemoryScope::GlobalPreferences && memory.kind == MemoryKind::Preference,
            "global preference store accepts only explicit global preference records"
        );
        ensure!(
            memory.evidence_ids.is_empty() && memory.supersedes.is_empty(),
            "global preferences cannot link project evidence or project memory versions"
        );
        let replay: Option<(String, String, String)> = self
            .connection
            .query_row(
                "SELECT preference_id, title, content FROM global_preferences WHERE version_id = ?1",
                [memory.version_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((preference_id, title, content)) = replay {
            ensure!(
                preference_id == memory.id.to_string()
                    && title == memory.title
                    && content == memory.content,
                "global preference version ID collision has different content"
            );
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version_number: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(version_number), 0) + 1 FROM global_preferences WHERE preference_id = ?1",
            [memory.id.to_string()],
            |row| row.get(0),
        )?;
        transaction.execute(
            r#"
            INSERT INTO global_preferences(
                preference_id, version_id, version_number, title, content,
                valid_from_ns, valid_to_ns, recorded_at_ns, confidence,
                authority, status, projection_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                memory.id.to_string(),
                memory.version_id.to_string(),
                version_number,
                memory.title,
                memory.content,
                timestamp_ns(memory.valid_from)?,
                memory.valid_to.map(timestamp_ns).transpose()?,
                timestamp_ns(memory.recorded_at)?,
                f64::from(memory.confidence),
                memory.authority.as_str(),
                memory.status.as_str(),
                memory.projection_path().to_string_lossy(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn versions(&self, preference_id: uuid::Uuid) -> Result<Vec<MemoryRecord>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT version_id, title, content, valid_from_ns, valid_to_ns,
                   recorded_at_ns, confidence, authority, status
            FROM global_preferences
            WHERE preference_id = ?1
            ORDER BY version_number ASC
            "#,
        )?;
        let rows = statement.query_map([preference_id.to_string()], |row| {
            Ok(RawGlobalPreference {
                version_id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                valid_from_ns: row.get(3)?,
                valid_to_ns: row.get(4)?,
                recorded_at_ns: row.get(5)?,
                confidence: row.get(6)?,
                authority: row.get(7)?,
                status: row.get(8)?,
            })
        })?;
        rows.map(|row| row?.parse(preference_id)).collect()
    }

    pub fn current_preferences(&self) -> Result<Vec<MemoryRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT preference_id FROM global_preferences ORDER BY preference_id",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let ids = rows
            .map(|row| Ok(uuid::Uuid::parse_str(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        let mut preferences = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(preference) = self.versions(id)?.pop()
                && !matches!(
                    preference.status,
                    MemoryStatus::Invalid | MemoryStatus::Superseded
                )
            {
                preferences.push(preference);
            }
        }
        Ok(preferences)
    }

    pub fn preference_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(DISTINCT preference_id) FROM global_preferences",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn note_import_hash(&self, note_path: &str) -> Result<Option<[u8; 32]>> {
        global_note_import_hash(&self.connection, note_path)
    }

    pub fn queue_note_review(
        &mut self,
        note_path: &str,
        content_hash: [u8; 32],
        reason: &str,
        observed_at: time::OffsetDateTime,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            r#"
            INSERT OR IGNORE INTO note_review_queue(
                project_id, note_path, content_hash, reason, observed_at_ns, resolved_at_ns
            ) VALUES ('global-preferences', ?1, ?2, ?3, ?4, NULL)
            "#,
            params![
                note_path,
                content_hash.as_slice(),
                reason.chars().take(500).collect::<String>(),
                timestamp_ns(observed_at)?,
            ],
        )? > 0)
    }

    pub fn record_note_promotion(
        &mut self,
        note_path: &str,
        content_hash: [u8; 32],
        preference_id: uuid::Uuid,
        version_id: uuid::Uuid,
        observed_at: time::OffsetDateTime,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            r#"
            INSERT INTO note_imports(
                project_id, note_path, content_hash, memory_id, version_id, imported_at_ns
            ) VALUES ('global-preferences', ?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(project_id, note_path) DO UPDATE SET
                content_hash = excluded.content_hash,
                memory_id = excluded.memory_id,
                version_id = excluded.version_id,
                imported_at_ns = excluded.imported_at_ns
            "#,
            params![
                note_path,
                content_hash.as_slice(),
                preference_id.to_string(),
                version_id.to_string(),
                timestamp_ns(observed_at)?,
            ],
        )?;
        transaction.execute(
            r#"
            INSERT OR IGNORE INTO global_preference_audit(
                audit_id, note_path, content_hash, preference_id,
                version_id, action, observed_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'explicit_promotion', ?6)
            "#,
            params![
                version_id.to_string(),
                note_path,
                content_hash.as_slice(),
                preference_id.to_string(),
                version_id.to_string(),
                timestamp_ns(observed_at)?,
            ],
        )?;
        transaction.execute(
            r#"
            UPDATE note_review_queue SET resolved_at_ns = ?2
            WHERE project_id = 'global-preferences' AND note_path = ?1
              AND resolved_at_ns IS NULL
            "#,
            params![note_path, timestamp_ns(observed_at)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn promotion_audit_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM global_preference_audit",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn note_review_count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM note_review_queue WHERE project_id = 'global-preferences' AND resolved_at_ns IS NULL",
            [],
            |row| row.get(0),
        )?)
    }
}

fn global_note_import_hash(connection: &Connection, note_path: &str) -> Result<Option<[u8; 32]>> {
    let bytes = connection
        .query_row(
            "SELECT content_hash FROM note_imports WHERE project_id = 'global-preferences' AND note_path = ?1",
            [note_path],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?;
    bytes
        .map(|bytes| {
            bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("stored global note hash is not 32 bytes"))
        })
        .transpose()
}

fn validate_record(memory: &MemoryRecord) -> Result<()> {
    ensure!(!memory.title.trim().is_empty(), "memory title is empty");
    ensure!(!memory.content.trim().is_empty(), "memory content is empty");
    ensure!(memory.title.len() <= 300, "memory title exceeds 300 bytes");
    ensure!(
        memory.content.len() <= 20_000,
        "memory content exceeds 20000 bytes"
    );
    ensure!(
        memory.confidence.is_finite() && (0.0..=1.0).contains(&memory.confidence),
        "memory confidence must be between zero and one"
    );
    if let Some(valid_to) = memory.valid_to {
        ensure!(
            valid_to > memory.valid_from,
            "memory validity interval is empty"
        );
    }
    Ok(())
}

fn load_project_versions(
    connection: &Connection,
    project_id: ProjectId,
    memory_id: uuid::Uuid,
) -> Result<Vec<MemoryRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT v.version_id, r.kind, v.worktree_id, v.task_id, v.title,
               v.content, v.valid_from_ns, v.valid_to_ns, v.recorded_at_ns,
               v.confidence, v.authority, v.status
        FROM memory_versions v
        JOIN memory_records r ON r.memory_id = v.memory_id
        WHERE v.memory_id = ?1 AND r.project_id = ?2
        ORDER BY v.version_number ASC
        "#,
    )?;
    let rows = statement.query_map(
        params![memory_id.to_string(), project_id.0.to_string()],
        |row| {
            Ok(RawProjectMemory {
                version_id: row.get(0)?,
                kind: row.get(1)?,
                worktree_id: row.get(2)?,
                task_id: row.get(3)?,
                title: row.get(4)?,
                content: row.get(5)?,
                valid_from_ns: row.get(6)?,
                valid_to_ns: row.get(7)?,
                recorded_at_ns: row.get(8)?,
                confidence: row.get(9)?,
                authority: row.get(10)?,
                status: row.get(11)?,
            })
        },
    )?;
    let raw = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter()
        .map(|raw| raw.parse(connection, memory_id, project_id))
        .collect()
}

struct RawProjectMemory {
    version_id: String,
    kind: String,
    worktree_id: Option<String>,
    task_id: Option<String>,
    title: String,
    content: String,
    valid_from_ns: i64,
    valid_to_ns: Option<i64>,
    recorded_at_ns: i64,
    confidence: f64,
    authority: String,
    status: String,
}

impl RawProjectMemory {
    fn parse(
        self,
        connection: &Connection,
        memory_id: uuid::Uuid,
        project_id: ProjectId,
    ) -> Result<MemoryRecord> {
        let version_id = uuid::Uuid::parse_str(&self.version_id)?;
        Ok(MemoryRecord {
            id: memory_id,
            version_id,
            scope: MemoryScope::Project(project_id),
            worktree_id: self
                .worktree_id
                .map(|value| uuid::Uuid::parse_str(&value).map(WorktreeId))
                .transpose()?,
            task_id: self
                .task_id
                .map(|value| uuid::Uuid::parse_str(&value))
                .transpose()?,
            kind: MemoryKind::from_name(&self.kind).context("stored memory kind is invalid")?,
            title: self.title,
            content: self.content,
            valid_from: from_ns(self.valid_from_ns)?,
            valid_to: self.valid_to_ns.map(from_ns).transpose()?,
            recorded_at: from_ns(self.recorded_at_ns)?,
            confidence: self.confidence as f32,
            authority: Authority::from_name(&self.authority)
                .context("stored memory authority is invalid")?,
            evidence_ids: load_ids(
                connection,
                "SELECT event_id FROM memory_evidence WHERE version_id = ?1 ORDER BY event_id",
                version_id,
            )?,
            supersedes: load_ids(
                connection,
                "SELECT superseded_version_id FROM memory_supersession WHERE version_id = ?1 ORDER BY superseded_version_id",
                version_id,
            )?,
            status: MemoryStatus::from_name(&self.status)
                .context("stored memory status is invalid")?,
        })
    }
}

struct RawGlobalPreference {
    version_id: String,
    title: String,
    content: String,
    valid_from_ns: i64,
    valid_to_ns: Option<i64>,
    recorded_at_ns: i64,
    confidence: f64,
    authority: String,
    status: String,
}

impl RawGlobalPreference {
    fn parse(self, preference_id: uuid::Uuid) -> Result<MemoryRecord> {
        Ok(MemoryRecord {
            id: preference_id,
            version_id: uuid::Uuid::parse_str(&self.version_id)?,
            scope: MemoryScope::GlobalPreferences,
            worktree_id: None,
            task_id: None,
            kind: MemoryKind::Preference,
            title: self.title,
            content: self.content,
            valid_from: from_ns(self.valid_from_ns)?,
            valid_to: self.valid_to_ns.map(from_ns).transpose()?,
            recorded_at: from_ns(self.recorded_at_ns)?,
            confidence: self.confidence as f32,
            authority: Authority::from_name(&self.authority)
                .context("stored preference authority is invalid")?,
            evidence_ids: Vec::new(),
            supersedes: Vec::new(),
            status: MemoryStatus::from_name(&self.status)
                .context("stored preference status is invalid")?,
        })
    }
}

fn load_ids(connection: &Connection, sql: &str, version_id: uuid::Uuid) -> Result<Vec<uuid::Uuid>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([version_id.to_string()], |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(uuid::Uuid::parse_str(&row?)?)).collect()
}

fn from_ns(value: i64) -> Result<time::OffsetDateTime> {
    Ok(time::OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(value),
    )?)
}
