use std::path::Path;

use anyhow::{Context, Result, ensure};
use brain_domain::{Harness, ProjectId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::migrations::{configure, migrate};

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderCacheEntry {
    pub provider: String,
    pub project_id: ProjectId,
    pub task_id: Option<uuid::Uuid>,
    pub query_sha256: [u8; 32],
    pub config_sha256: [u8; 32],
    pub source_version: String,
    pub fetched_at: time::OffsetDateTime,
    pub expires_at: time::OffsetDateTime,
    pub items: serde_json::Value,
}

pub struct ProviderCacheStore {
    connection: Connection,
    project_id: ProjectId,
}

impl ProviderCacheStore {
    pub fn open(path: impl AsRef<Path>, project_id: ProjectId) -> Result<Self> {
        let connection = Connection::open(path)?;
        configure(&connection)?;
        migrate(&connection)?;
        Ok(Self {
            connection,
            project_id,
        })
    }

    pub fn put(&mut self, entry: &ProviderCacheEntry) -> Result<()> {
        ensure!(
            entry.project_id == self.project_id,
            "provider cache violates project scope"
        );
        ensure!(
            !entry.provider.trim().is_empty(),
            "provider cache name is empty"
        );
        ensure!(
            entry.expires_at > entry.fetched_at,
            "provider cache expiry is invalid"
        );
        self.connection.execute(
            r#"
            INSERT INTO provider_cache(
                provider, project_id, task_id, query_sha256, config_sha256,
                source_version, fetched_at_ns, expires_at_ns, items_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(provider, project_id, query_sha256, config_sha256, source_version)
            DO UPDATE SET task_id = excluded.task_id,
                fetched_at_ns = excluded.fetched_at_ns,
                expires_at_ns = excluded.expires_at_ns,
                items_json = excluded.items_json
            "#,
            params![
                entry.provider,
                entry.project_id.0.to_string(),
                entry.task_id.map(|id| id.to_string()),
                entry.query_sha256.as_slice(),
                entry.config_sha256.as_slice(),
                entry.source_version,
                timestamp(entry.fetched_at)?,
                timestamp(entry.expires_at)?,
                serde_json::to_string(&entry.items)?,
            ],
        )?;
        Ok(())
    }

    pub fn get(
        &self,
        provider: &str,
        query_sha256: [u8; 32],
        config_sha256: [u8; 32],
        source_version: &str,
        now: time::OffsetDateTime,
    ) -> Result<Option<ProviderCacheEntry>> {
        let raw = self
            .connection
            .query_row(
                r#"
            SELECT task_id, fetched_at_ns, expires_at_ns, items_json
            FROM provider_cache
            WHERE provider = ?1 AND project_id = ?2 AND query_sha256 = ?3
              AND config_sha256 = ?4 AND source_version = ?5 AND expires_at_ns > ?6
            "#,
                params![
                    provider,
                    self.project_id.0.to_string(),
                    query_sha256.as_slice(),
                    config_sha256.as_slice(),
                    source_version,
                    timestamp(now)?,
                ],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        raw.map(|(task, fetched, expires, items)| {
            Ok(ProviderCacheEntry {
                provider: provider.to_owned(),
                project_id: self.project_id,
                task_id: task.map(|id| uuid::Uuid::parse_str(&id)).transpose()?,
                query_sha256,
                config_sha256,
                source_version: source_version.to_owned(),
                fetched_at: from_timestamp(fetched)?,
                expires_at: from_timestamp(expires)?,
                items: serde_json::from_str(&items)?,
            })
        })
        .transpose()
    }

    pub fn latest(
        &self,
        provider: &str,
        config_sha256: [u8; 32],
        now: time::OffsetDateTime,
    ) -> Result<Option<ProviderCacheEntry>> {
        let raw = self
            .connection
            .query_row(
                r#"
            SELECT task_id, query_sha256, source_version, fetched_at_ns, expires_at_ns, items_json
            FROM provider_cache
            WHERE provider = ?1 AND project_id = ?2 AND config_sha256 = ?3
              AND expires_at_ns > ?4
            ORDER BY fetched_at_ns DESC LIMIT 1
            "#,
                params![
                    provider,
                    self.project_id.0.to_string(),
                    config_sha256.as_slice(),
                    timestamp(now)?
                ],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(|(task, query, source, fetched, expires, items)| {
            Ok(ProviderCacheEntry {
                provider: provider.to_owned(),
                project_id: self.project_id,
                task_id: task.map(|id| uuid::Uuid::parse_str(&id)).transpose()?,
                query_sha256: query
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid cache query hash"))?,
                config_sha256,
                source_version: source,
                fetched_at: from_timestamp(fetched)?,
                expires_at: from_timestamp(expires)?,
                items: serde_json::from_str(&items)?,
            })
        })
        .transpose()
    }

    pub fn claim_first_prompt(
        &mut self,
        harness: &Harness,
        native_session_id: &str,
        prompt_sha256: [u8; 32],
        now: time::OffsetDateTime,
    ) -> Result<bool> {
        ensure!(
            !native_session_id.trim().is_empty(),
            "native session ID is empty"
        );
        Ok(self.connection.execute(
            r#"
            INSERT OR IGNORE INTO provider_prompt_state(
                project_id, harness, native_session_id, prompt_sha256, claimed_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                self.project_id.0.to_string(),
                harness.as_str(),
                native_session_id,
                prompt_sha256.as_slice(),
                timestamp(now)?,
            ],
        )? == 1)
    }

    pub fn remove_provider_cache(&mut self, provider: &str) -> Result<u64> {
        Ok(u64::try_from(self.connection.execute(
            "DELETE FROM provider_cache WHERE project_id = ?1 AND provider = ?2",
            params![self.project_id.0.to_string(), provider],
        )?)?)
    }
}

fn timestamp(value: time::OffsetDateTime) -> Result<i64> {
    Ok(i64::try_from(value.unix_timestamp_nanos())?)
}

fn from_timestamp(value: i64) -> Result<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(value))
        .context("invalid provider cache timestamp")
}
