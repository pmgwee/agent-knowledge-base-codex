use std::path::{Path, PathBuf};

use anyhow::Result;
use brain_domain::ProjectId;
use rusqlite::{Connection, params};

use crate::migrations::{configure, migrate};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CatalogEvent {
    pub event_id: uuid::Uuid,
    pub segment_id: uuid::Uuid,
    pub line_number: u64,
    pub raw_hash: [u8; 32],
}

pub struct SegmentCatalog {
    connection: Connection,
    project_id: ProjectId,
}

impl SegmentCatalog {
    pub fn open(path: impl AsRef<Path>, project_id: ProjectId) -> Result<Self> {
        let connection = Connection::open(path)?;
        configure(&connection)?;
        migrate(&connection)?;
        Ok(Self {
            connection,
            project_id,
        })
    }

    pub fn events(&self, segment_id: uuid::Uuid) -> Result<Vec<CatalogEvent>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT event_id, segment_id, line_number, raw_hash
            FROM event_segment_catalog
            WHERE project_id = ?1 AND segment_id = ?2
            ORDER BY line_number ASC
            "#,
        )?;
        let rows = statement.query_map(
            params![self.project_id.0.to_string(), segment_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (event, segment, line, hash) = row?;
            Ok(CatalogEvent {
                event_id: uuid::Uuid::parse_str(&event)?,
                segment_id: uuid::Uuid::parse_str(&segment)?,
                line_number: u64::try_from(line)?,
                raw_hash: hash
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid raw hash"))?,
            })
        })
        .collect()
    }

    pub fn manifest_paths(&self) -> Result<Vec<PathBuf>> {
        let mut statement = self.connection.prepare(
            "SELECT manifest_path FROM sealed_segments WHERE project_id = ?1 ORDER BY occurred_min_ns",
        )?;
        let rows = statement.query_map([self.project_id.0.to_string()], |row| {
            row.get::<_, String>(0).map(PathBuf::from)
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
