use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, ensure};
use brain_domain::ProjectId;
use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::migrations::{configure, migrate};
use crate::{EventLedger, StoredEvent};

pub const SEGMENT_FORMAT_VERSION: u32 = 1;
pub const SEGMENT_SIZE_THRESHOLD: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SegmentManifest {
    pub format_version: u32,
    pub project_id: ProjectId,
    pub segment_id: uuid::Uuid,
    pub first_event_id: uuid::Uuid,
    pub last_event_id: uuid::Uuid,
    pub event_count: u64,
    pub occurred_min: time::OffsetDateTime,
    pub occurred_max: time::OffsetDateTime,
    pub compressed_sha256: [u8; 32],
    pub uncompressed_sha256: [u8; 32],
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
    pub data_file: String,
    pub published_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SegmentSealReport {
    pub manifest: SegmentManifest,
    pub manifest_path: PathBuf,
    pub data_path: PathBuf,
    pub replayed: bool,
}

pub struct SegmentStore {
    ledger_path: PathBuf,
    root: PathBuf,
    project_id: ProjectId,
    segment_open_count: Arc<AtomicU64>,
}

impl SegmentStore {
    pub fn new(
        ledger_path: impl AsRef<Path>,
        segment_root: impl AsRef<Path>,
        project_id: ProjectId,
    ) -> Result<Self> {
        fs::create_dir_all(segment_root.as_ref())?;
        Ok(Self {
            ledger_path: ledger_path.as_ref().to_path_buf(),
            root: segment_root.as_ref().to_path_buf(),
            project_id,
            segment_open_count: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn seal(
        &self,
        first_event_id: uuid::Uuid,
        last_event_id: uuid::Uuid,
        published_at: time::OffsetDateTime,
    ) -> Result<SegmentSealReport> {
        let connection = Connection::open(&self.ledger_path)?;
        configure(&connection)?;
        migrate(&connection)?;
        if let Some(report) = self.existing(&connection, first_event_id, last_event_id)? {
            return Ok(report);
        }
        drop(connection);

        let ledger = EventLedger::open(&self.ledger_path, self.project_id)?;
        let events = ledger.events_between(first_event_id, last_event_id)?;
        ensure!(!events.is_empty(), "cannot seal an empty event range");
        ensure!(
            events
                .first()
                .is_some_and(|event| event.event_id == first_event_id)
                && events
                    .last()
                    .is_some_and(|event| event.event_id == last_event_id),
            "segment range endpoints are not in canonical event order"
        );
        ensure!(
            events
                .iter()
                .all(|event| event.project_id == self.project_id),
            "segment range violates project scope"
        );
        let mut uncompressed = Vec::new();
        for event in &events {
            serde_json::to_writer(&mut uncompressed, event)?;
            uncompressed.push(b'\n');
        }
        let compressed = zstd::stream::encode_all(uncompressed.as_slice(), 3)?;
        let uncompressed_sha256: [u8; 32] = Sha256::digest(&uncompressed).into();
        let compressed_sha256: [u8; 32] = Sha256::digest(&compressed).into();
        let segment_id = deterministic_segment_id(self.project_id, &uncompressed_sha256);
        let stem = format!("{}-{}", published_at.date(), hex::encode(compressed_sha256));
        let data_path = self.root.join(format!("{stem}.jsonl.zst"));
        let manifest_path = self.root.join(format!("{stem}.manifest.json"));
        let data_file = data_path
            .file_name()
            .expect("segment data has filename")
            .to_string_lossy()
            .to_string();
        let manifest = SegmentManifest {
            format_version: SEGMENT_FORMAT_VERSION,
            project_id: self.project_id,
            segment_id,
            first_event_id,
            last_event_id,
            event_count: u64::try_from(events.len())?,
            occurred_min: events.iter().map(|event| event.occurred_at).min().unwrap(),
            occurred_max: events.iter().map(|event| event.occurred_at).max().unwrap(),
            compressed_sha256,
            uncompressed_sha256,
            compressed_bytes: u64::try_from(compressed.len())?,
            uncompressed_bytes: u64::try_from(uncompressed.len())?,
            data_file,
            published_at,
        };

        write_create_new(&data_path, &compressed)?;
        if let Err(error) = verify_data(&data_path, &manifest) {
            let _ = fs::remove_file(&data_path);
            return Err(error);
        }
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        write_create_new(&manifest_path, &manifest_bytes)?;

        let mut connection = Connection::open(&self.ledger_path)?;
        configure(&connection)?;
        migrate(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            r#"
            INSERT INTO sealed_segments(
                segment_id, project_id, manifest_path, data_path, first_event_id,
                last_event_id, event_count, occurred_min_ns, occurred_max_ns,
                compressed_sha256, uncompressed_sha256, compressed_bytes,
                uncompressed_bytes, published_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                manifest.segment_id.to_string(),
                manifest.project_id.0.to_string(),
                manifest_path.to_string_lossy(),
                data_path.to_string_lossy(),
                manifest.first_event_id.to_string(),
                manifest.last_event_id.to_string(),
                i64::try_from(manifest.event_count)?,
                i64::try_from(manifest.occurred_min.unix_timestamp_nanos())?,
                i64::try_from(manifest.occurred_max.unix_timestamp_nanos())?,
                manifest.compressed_sha256.as_slice(),
                manifest.uncompressed_sha256.as_slice(),
                i64::try_from(manifest.compressed_bytes)?,
                i64::try_from(manifest.uncompressed_bytes)?,
                i64::try_from(manifest.published_at.unix_timestamp_nanos())?,
            ],
        )?;
        for (line_number, event) in events.iter().enumerate() {
            let raw_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(&event.raw)?).into();
            transaction.execute(
                r#"
                INSERT INTO event_segment_catalog(
                    event_id, project_id, segment_id, line_number, raw_hash, search_text, path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    event.event_id.to_string(),
                    self.project_id.0.to_string(),
                    segment_id.to_string(),
                    i64::try_from(line_number)?,
                    raw_hash.as_slice(),
                    serde_json::to_string(&event.payload)?,
                    event
                        .payload
                        .get("path")
                        .or_else(|| event.payload.get("file_path"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(&event.source_locator),
                ],
            )?;
        }
        transaction.commit()?;
        Ok(SegmentSealReport {
            manifest,
            manifest_path,
            data_path,
            replayed: false,
        })
    }

    pub fn read(&self, manifest_path: impl AsRef<Path>) -> Result<Vec<StoredEvent>> {
        let manifest: SegmentManifest = serde_json::from_slice(&fs::read(manifest_path.as_ref())?)?;
        ensure!(
            manifest.project_id == self.project_id,
            "segment project mismatch"
        );
        let data_path = manifest_path
            .as_ref()
            .parent()
            .context("segment manifest has no parent")?
            .join(&manifest.data_file);
        self.segment_open_count.fetch_add(1, Ordering::Relaxed);
        let uncompressed = verify_data(&data_path, &manifest)?;
        uncompressed
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(Into::into))
            .collect()
    }

    pub fn event(&self, event_id: uuid::Uuid) -> Result<Option<StoredEvent>> {
        use rusqlite::OptionalExtension;
        let connection = Connection::open(&self.ledger_path)?;
        configure(&connection)?;
        migrate(&connection)?;
        let location = connection
            .query_row(
                r#"
                SELECT s.manifest_path, c.line_number
                FROM event_segment_catalog c
                JOIN sealed_segments s ON s.segment_id = c.segment_id
                WHERE c.project_id = ?1 AND c.event_id = ?2
                "#,
                params![self.project_id.0.to_string(), event_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((manifest, line)) = location else {
            return Ok(None);
        };
        let line = usize::try_from(line)?;
        Ok(self.read(manifest)?.into_iter().nth(line))
    }

    pub fn compact(&self, segment_id: uuid::Uuid) -> Result<u64> {
        let mut connection = Connection::open(&self.ledger_path)?;
        configure(&connection)?;
        migrate(&connection)?;
        let manifest_path: String = connection.query_row(
            "SELECT manifest_path FROM sealed_segments WHERE project_id = ?1 AND segment_id = ?2",
            params![self.project_id.0.to_string(), segment_id.to_string()],
            |row| row.get(0),
        )?;
        let events = self.read(&manifest_path)?;
        ensure!(!events.is_empty(), "verified segment is empty");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pending: i64 = transaction.query_row(
            r#"
            SELECT COUNT(*) FROM consolidation_jobs j
            JOIN events first_event ON first_event.event_id = j.first_event_id
            JOIN events last_event ON last_event.event_id = j.last_event_id
            JOIN event_segment_catalog c ON c.segment_id = ?2
            JOIN events candidate ON candidate.event_id = c.event_id
            WHERE j.project_id = ?1 AND j.status != 'completed'
              AND candidate.rowid BETWEEN first_event.rowid AND last_event.rowid
            "#,
            params![self.project_id.0.to_string(), segment_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            pending == 0,
            "cannot compact a segment needed by pending consolidation jobs"
        );
        let changed = transaction.execute(
            r#"
            UPDATE events SET payload_json = '{}', raw_json = '{}', archived = 1
            WHERE project_id = ?1 AND event_id IN (
                SELECT event_id FROM event_segment_catalog WHERE project_id = ?1 AND segment_id = ?2
            )
            "#,
            params![self.project_id.0.to_string(), segment_id.to_string()],
        )?;
        ensure!(
            changed == events.len(),
            "compaction event count differs from verified segment"
        );
        transaction.commit()?;
        Ok(u64::try_from(changed)?)
    }

    pub fn segment_open_count(&self) -> u64 {
        self.segment_open_count.load(Ordering::Relaxed)
    }

    fn existing(
        &self,
        connection: &Connection,
        first: uuid::Uuid,
        last: uuid::Uuid,
    ) -> Result<Option<SegmentSealReport>> {
        use rusqlite::OptionalExtension;
        let paths = connection
            .query_row(
                r#"
                SELECT manifest_path, data_path FROM sealed_segments
                WHERE project_id = ?1 AND first_event_id = ?2 AND last_event_id = ?3
                "#,
                params![
                    self.project_id.0.to_string(),
                    first.to_string(),
                    last.to_string()
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((manifest_path, data_path)) = paths else {
            return Ok(None);
        };
        let manifest_path = PathBuf::from(manifest_path);
        let data_path = PathBuf::from(data_path);
        let manifest: SegmentManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        verify_data(&data_path, &manifest)?;
        Ok(Some(SegmentSealReport {
            manifest,
            manifest_path,
            data_path,
            replayed: true,
        }))
    }
}

pub fn should_seal(
    ledger_bytes: u64,
    oldest_event: time::OffsetDateTime,
    now: time::OffsetDateTime,
) -> bool {
    ledger_bytes >= SEGMENT_SIZE_THRESHOLD
        || (oldest_event.year() != now.year() || oldest_event.month() != now.month())
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    Ok(())
}

fn verify_data(path: &Path, manifest: &SegmentManifest) -> Result<Vec<u8>> {
    let compressed = fs::read(path).with_context(|| format!("read segment {}", path.display()))?;
    ensure!(
        <[u8; 32]>::from(Sha256::digest(&compressed)) == manifest.compressed_sha256,
        "compressed segment checksum mismatch"
    );
    let uncompressed = zstd::stream::decode_all(compressed.as_slice())?;
    ensure!(
        <[u8; 32]>::from(Sha256::digest(&uncompressed)) == manifest.uncompressed_sha256,
        "uncompressed segment checksum mismatch"
    );
    ensure!(
        uncompressed
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .count()
            == usize::try_from(manifest.event_count)?,
        "segment event count mismatch"
    );
    Ok(uncompressed)
}

fn deterministic_segment_id(project: ProjectId, hash: &[u8; 32]) -> uuid::Uuid {
    let mut digest = Sha256::new();
    digest.update(project.0.as_bytes());
    digest.update(hash);
    let mut bytes: [u8; 16] = digest.finalize()[..16].try_into().expect("digest slice");
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}
