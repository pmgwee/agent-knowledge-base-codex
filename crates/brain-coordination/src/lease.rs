use brain_domain::{Harness, WorktreeId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{CoordinationStore, DEFAULT_LEASE_DURATION, from_ns, timestamp_ns};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SessionIdentity {
    pub harness: Harness,
    pub native_session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WriterLease {
    pub task_id: uuid::Uuid,
    pub worktree_id: WorktreeId,
    pub owner: SessionIdentity,
    pub acquired_at: time::OffsetDateTime,
    pub renewed_at: time::OffsetDateTime,
    pub expires_at: time::OffsetDateTime,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CoordinationEvent {
    pub id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub worktree_id: WorktreeId,
    pub action: String,
    pub owner: Option<SessionIdentity>,
    pub prior_owner: Option<SessionIdentity>,
    pub generation: Option<u64>,
    pub occurred_at: time::OffsetDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("writer lease is held by {owner_harness}/{owner_session} until {expires_at}")]
    AlreadyHeld {
        owner_harness: String,
        owner_session: String,
        expires_at: time::OffsetDateTime,
    },
    #[error("writer lease owner does not match the requesting session")]
    WrongOwner,
    #[error("writer lease generation changed; refresh status before retrying")]
    GenerationChanged,
    #[error("task does not exist or is not active")]
    TaskUnavailable,
    #[error("lease duration must be positive")]
    InvalidDuration,
    #[error(transparent)]
    Store(#[from] anyhow::Error),
}

impl CoordinationStore {
    pub fn acquire_lease(
        &mut self,
        task_id: uuid::Uuid,
        owner: SessionIdentity,
        now: time::OffsetDateTime,
        duration: Option<time::Duration>,
    ) -> Result<WriterLease, LeaseError> {
        let duration = duration.unwrap_or(DEFAULT_LEASE_DURATION);
        if !duration.is_positive() {
            return Err(LeaseError::InvalidDuration);
        }
        let project = self.project_id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)?;
        let worktree_id = active_task_worktree(&transaction, project, task_id)?;
        let existing = load_lease(&transaction, project, task_id)?;
        let lease = match existing {
            Some(existing) if existing.expires_at > now && existing.owner != owner => {
                return Err(LeaseError::AlreadyHeld {
                    owner_harness: existing.owner.harness.as_str().to_owned(),
                    owner_session: existing.owner.native_session_id,
                    expires_at: existing.expires_at,
                });
            }
            Some(existing) if existing.expires_at > now => {
                let renewed_at = now.max(existing.renewed_at);
                let renewed = WriterLease {
                    expires_at: renewed_at + duration,
                    renewed_at,
                    ..existing
                };
                store_lease(&transaction, project, &renewed)?;
                append_event(&transaction, project, "renew", &renewed, None, now)?;
                renewed
            }
            Some(existing) => {
                let lease = WriterLease {
                    task_id,
                    worktree_id,
                    owner,
                    acquired_at: now,
                    renewed_at: now,
                    expires_at: now + duration,
                    generation: existing.generation.saturating_add(1),
                };
                store_lease(&transaction, project, &lease)?;
                append_event(
                    &transaction,
                    project,
                    "takeover",
                    &lease,
                    Some(&existing.owner),
                    now,
                )?;
                lease
            }
            None => {
                let lease = WriterLease {
                    task_id,
                    worktree_id,
                    owner,
                    acquired_at: now,
                    renewed_at: now,
                    expires_at: now + duration,
                    generation: 1,
                };
                store_lease(&transaction, project, &lease)?;
                append_event(&transaction, project, "acquire", &lease, None, now)?;
                lease
            }
        };
        transaction.commit().map_err(anyhow::Error::from)?;
        Ok(lease)
    }

    pub fn renew_lease(
        &mut self,
        task_id: uuid::Uuid,
        owner: &SessionIdentity,
        generation: u64,
        now: time::OffsetDateTime,
        duration: Option<time::Duration>,
    ) -> Result<WriterLease, LeaseError> {
        let duration = duration.unwrap_or(DEFAULT_LEASE_DURATION);
        if !duration.is_positive() {
            return Err(LeaseError::InvalidDuration);
        }
        let project = self.project_id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)?;
        let existing =
            load_lease(&transaction, project, task_id)?.ok_or(LeaseError::TaskUnavailable)?;
        if &existing.owner != owner {
            return Err(LeaseError::WrongOwner);
        }
        if existing.generation != generation {
            return Err(LeaseError::GenerationChanged);
        }
        let renewed_at = now.max(existing.renewed_at);
        let renewed = WriterLease {
            renewed_at,
            expires_at: renewed_at + duration,
            ..existing
        };
        store_lease(&transaction, project, &renewed)?;
        append_event(&transaction, project, "renew", &renewed, None, now)?;
        transaction.commit().map_err(anyhow::Error::from)?;
        Ok(renewed)
    }

    pub fn release_lease(
        &mut self,
        task_id: uuid::Uuid,
        owner: &SessionIdentity,
        generation: u64,
        now: time::OffsetDateTime,
    ) -> Result<(), LeaseError> {
        let project = self.project_id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)?;
        let existing =
            load_lease(&transaction, project, task_id)?.ok_or(LeaseError::TaskUnavailable)?;
        if &existing.owner != owner {
            return Err(LeaseError::WrongOwner);
        }
        if existing.generation != generation {
            return Err(LeaseError::GenerationChanged);
        }
        transaction
            .execute(
                "DELETE FROM writer_leases WHERE project_id = ?1 AND task_id = ?2 AND generation = ?3",
                params![project.0.to_string(), task_id.to_string(), i64::try_from(generation).map_err(anyhow::Error::from)?],
            )
            .map_err(anyhow::Error::from)?;
        append_event(&transaction, project, "release", &existing, None, now)?;
        transaction.commit().map_err(anyhow::Error::from)?;
        Ok(())
    }

    pub fn handoff_lease(
        &mut self,
        task_id: uuid::Uuid,
        current_owner: &SessionIdentity,
        generation: u64,
        next_owner: SessionIdentity,
        now: time::OffsetDateTime,
    ) -> Result<WriterLease, LeaseError> {
        let project = self.project_id;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(anyhow::Error::from)?;
        let existing =
            load_lease(&transaction, project, task_id)?.ok_or(LeaseError::TaskUnavailable)?;
        if &existing.owner != current_owner {
            return Err(LeaseError::WrongOwner);
        }
        if existing.generation != generation {
            return Err(LeaseError::GenerationChanged);
        }
        let lease = WriterLease {
            owner: next_owner,
            acquired_at: now,
            renewed_at: now,
            expires_at: now + DEFAULT_LEASE_DURATION,
            generation: generation.saturating_add(1),
            ..existing.clone()
        };
        store_lease(&transaction, project, &lease)?;
        append_event(
            &transaction,
            project,
            "handoff",
            &lease,
            Some(&existing.owner),
            now,
        )?;
        transaction.commit().map_err(anyhow::Error::from)?;
        Ok(lease)
    }

    pub fn active_leases(&self, now: time::OffsetDateTime) -> anyhow::Result<Vec<WriterLease>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT task_id, worktree_id, owner_harness, owner_session,
                   acquired_at_ns, renewed_at_ns, expires_at_ns, generation
            FROM writer_leases
            WHERE project_id = ?1 AND expires_at_ns > ?2
            ORDER BY expires_at_ns ASC, task_id ASC
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_id.0.to_string(), timestamp_ns(now)?],
            parse_lease,
        )?;
        rows.map(|row| parse_lease_record(row?)).collect()
    }

    pub fn lease(&self, task_id: uuid::Uuid) -> anyhow::Result<Option<WriterLease>> {
        load_lease(&self.connection, self.project_id, task_id)
    }

    pub fn coordination_events(&self) -> anyhow::Result<Vec<CoordinationEvent>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, task_id, worktree_id, action, owner_harness,
                   owner_session, prior_owner_harness, prior_owner_session,
                   generation, occurred_at_ns
            FROM coordination_events
            WHERE project_id = ?1
            ORDER BY occurred_at_ns ASC, event_id ASC
            "#,
        )?;
        let rows = statement.query_map([self.project_id.0.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })?;
        rows.map(|row| {
            let raw = row?;
            Ok(CoordinationEvent {
                id: uuid::Uuid::parse_str(&raw.0)?,
                task_id: uuid::Uuid::parse_str(&raw.1)?,
                worktree_id: WorktreeId(uuid::Uuid::parse_str(&raw.2)?),
                action: raw.3,
                owner: session(raw.4, raw.5),
                prior_owner: session(raw.6, raw.7),
                generation: raw.8.map(u64::try_from).transpose()?,
                occurred_at: from_ns(raw.9)?,
            })
        })
        .collect()
    }
}

fn active_task_worktree(
    transaction: &Transaction<'_>,
    project: brain_domain::ProjectId,
    task_id: uuid::Uuid,
) -> Result<WorktreeId, LeaseError> {
    let id = transaction
        .query_row(
            "SELECT worktree_id FROM coordination_tasks WHERE project_id = ?1 AND task_id = ?2 AND status = 'active'",
            params![project.0.to_string(), task_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)?
        .ok_or(LeaseError::TaskUnavailable)?;
    Ok(WorktreeId(
        uuid::Uuid::parse_str(&id).map_err(anyhow::Error::from)?,
    ))
}

fn store_lease(
    transaction: &Transaction<'_>,
    project: brain_domain::ProjectId,
    lease: &WriterLease,
) -> Result<(), LeaseError> {
    transaction
        .execute(
            r#"
            INSERT INTO writer_leases(
                task_id, project_id, worktree_id, owner_harness, owner_session,
                acquired_at_ns, renewed_at_ns, expires_at_ns, generation
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(task_id) DO UPDATE SET
                worktree_id = excluded.worktree_id,
                owner_harness = excluded.owner_harness,
                owner_session = excluded.owner_session,
                acquired_at_ns = excluded.acquired_at_ns,
                renewed_at_ns = excluded.renewed_at_ns,
                expires_at_ns = excluded.expires_at_ns,
                generation = excluded.generation
            "#,
            params![
                lease.task_id.to_string(),
                project.0.to_string(),
                lease.worktree_id.0.to_string(),
                lease.owner.harness.as_str(),
                lease.owner.native_session_id,
                timestamp_ns(lease.acquired_at)?,
                timestamp_ns(lease.renewed_at)?,
                timestamp_ns(lease.expires_at)?,
                i64::try_from(lease.generation).map_err(anyhow::Error::from)?,
            ],
        )
        .map_err(anyhow::Error::from)?;
    Ok(())
}

fn append_event(
    transaction: &Transaction<'_>,
    project: brain_domain::ProjectId,
    action: &str,
    lease: &WriterLease,
    prior_owner: Option<&SessionIdentity>,
    now: time::OffsetDateTime,
) -> Result<(), LeaseError> {
    transaction
        .execute(
            r#"
            INSERT INTO coordination_events(
                event_id, project_id, task_id, worktree_id, action,
                owner_harness, owner_session, prior_owner_harness,
                prior_owner_session, generation, occurred_at_ns, details_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, '{}')
            "#,
            params![
                uuid::Uuid::now_v7().to_string(),
                project.0.to_string(),
                lease.task_id.to_string(),
                lease.worktree_id.0.to_string(),
                action,
                lease.owner.harness.as_str(),
                lease.owner.native_session_id,
                prior_owner.map(|owner| owner.harness.as_str()),
                prior_owner.map(|owner| owner.native_session_id.as_str()),
                i64::try_from(lease.generation).map_err(anyhow::Error::from)?,
                timestamp_ns(now)?,
            ],
        )
        .map_err(anyhow::Error::from)?;
    Ok(())
}

type RawLease = (String, String, String, String, i64, i64, i64, i64);

fn load_lease(
    connection: &rusqlite::Connection,
    project: brain_domain::ProjectId,
    task_id: uuid::Uuid,
) -> anyhow::Result<Option<WriterLease>> {
    connection
        .query_row(
            r#"
            SELECT task_id, worktree_id, owner_harness, owner_session,
                   acquired_at_ns, renewed_at_ns, expires_at_ns, generation
            FROM writer_leases
            WHERE project_id = ?1 AND task_id = ?2
            "#,
            params![project.0.to_string(), task_id.to_string()],
            parse_lease,
        )
        .optional()?
        .map(parse_lease_record)
        .transpose()
}

fn parse_lease(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawLease> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn parse_lease_record(raw: RawLease) -> anyhow::Result<WriterLease> {
    Ok(WriterLease {
        task_id: uuid::Uuid::parse_str(&raw.0)?,
        worktree_id: WorktreeId(uuid::Uuid::parse_str(&raw.1)?),
        owner: SessionIdentity {
            harness: harness(&raw.2),
            native_session_id: raw.3,
        },
        acquired_at: from_ns(raw.4)?,
        renewed_at: from_ns(raw.5)?,
        expires_at: from_ns(raw.6)?,
        generation: u64::try_from(raw.7)?,
    })
}

fn harness(value: &str) -> Harness {
    match value {
        "claude-code" => Harness::ClaudeCode,
        "codex" => Harness::Codex,
        "hermes" => Harness::Hermes,
        other => Harness::Other(other.to_owned()),
    }
}

fn session(harness_value: Option<String>, session: Option<String>) -> Option<SessionIdentity> {
    Some(SessionIdentity {
        harness: harness(&harness_value?),
        native_session_id: session?,
    })
}
