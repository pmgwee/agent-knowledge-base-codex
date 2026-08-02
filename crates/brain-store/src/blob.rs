use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::ProjectId;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::migrations::{configure, migrate};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BlobRecord {
    pub raw_sha256: [u8; 32],
    pub mime: String,
    pub raw_bytes: u64,
    pub compressed_bytes: u64,
    pub path: PathBuf,
}

pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(brain_home: impl AsRef<Path>) -> Result<Self> {
        let root = brain_home.as_ref().join("blobs").join("sha256");
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn put(
        &self,
        ledger_path: impl AsRef<Path>,
        project_id: ProjectId,
        mime: &str,
        bytes: &[u8],
    ) -> Result<BlobRecord> {
        ensure!(!mime.trim().is_empty(), "blob MIME type is required");
        let raw_sha256: [u8; 32] = Sha256::digest(bytes).into();
        let hash = hex::encode(raw_sha256);
        let directory = self.root.join(&hash[..2]);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{hash}.zst"));
        if !path.exists() {
            let compressed = zstd::stream::encode_all(bytes, 3)?;
            let temporary = directory.join(format!(".{hash}.{}.tmp", uuid::Uuid::now_v7()));
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            output.write_all(&compressed)?;
            output.sync_all()?;
            drop(output);
            match fs::rename(&temporary, &path) {
                Ok(()) => {}
                Err(error) if path.exists() => {
                    let _ = fs::remove_file(&temporary);
                    let _ = error;
                }
                Err(error) => return Err(error.into()),
            }
        }
        let compressed_bytes = fs::metadata(&path)?.len();
        let connection = Connection::open(ledger_path.as_ref())?;
        configure(&connection)?;
        migrate(&connection)?;
        connection.execute(
            r#"
            INSERT OR IGNORE INTO project_blob_refs(
                project_id, raw_sha256, mime, raw_bytes, compressed_bytes, created_at_ns
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                project_id.0.to_string(),
                raw_sha256.as_slice(),
                mime,
                i64::try_from(bytes.len())?,
                i64::try_from(compressed_bytes)?,
                i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos())?,
            ],
        )?;
        Ok(BlobRecord {
            raw_sha256,
            mime: mime.to_owned(),
            raw_bytes: u64::try_from(bytes.len())?,
            compressed_bytes,
            path,
        })
    }

    pub fn read(&self, record: &BlobRecord) -> Result<Vec<u8>> {
        let compressed = fs::read(&record.path)
            .with_context(|| format!("read blob {}", record.path.display()))?;
        let bytes = zstd::stream::decode_all(compressed.as_slice())?;
        ensure!(
            <[u8; 32]>::from(Sha256::digest(&bytes)) == record.raw_sha256,
            "blob checksum mismatch"
        );
        Ok(bytes)
    }

    pub fn physical_count(&self) -> Result<u64> {
        let mut count = 0_u64;
        for prefix in fs::read_dir(&self.root)? {
            let prefix = prefix?;
            if !prefix.file_type()?.is_dir() {
                continue;
            }
            count += fs::read_dir(prefix.path())?
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|value| value == "zst"))
                .count() as u64;
        }
        Ok(count)
    }
}
