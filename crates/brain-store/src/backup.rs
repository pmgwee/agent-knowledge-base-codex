use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, DatabaseName};
use sha2::{Digest, Sha256};

pub const BACKUP_FORMAT_VERSION: u32 = 1;
const LEDGER_SCHEMA_VERSION: u32 = 9;

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryKind {
    Sqlite,
    Segment,
    Blob,
    Configuration,
    Projection,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct InventoryFile {
    pub relative_path: PathBuf,
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub kind: InventoryKind,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BackupInventory {
    pub format_version: u32,
    pub backup_id: uuid::Uuid,
    pub created_at: time::OffsetDateTime,
    pub binary_version: String,
    pub files: Vec<InventoryFile>,
    pub inventory_sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BackupReport {
    pub backup_path: PathBuf,
    pub inventory: BackupInventory,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VerificationReport {
    pub backup_path: PathBuf,
    pub file_count: u64,
    pub total_bytes: u64,
    pub sqlite_integrity_checks: u64,
    pub verified_at: time::OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RestoreReport {
    pub destination: PathBuf,
    pub inventory_sha256: [u8; 32],
    pub restored_files: u64,
    pub restored_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RetentionPolicy {
    pub hourly: usize,
    pub daily: usize,
    pub monthly: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            hourly: 24,
            daily: 30,
            monthly: 12,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RetentionReport {
    pub backup_root: PathBuf,
    pub retained: Vec<PathBuf>,
    pub pruned: Vec<PathBuf>,
    pub ignored: Vec<PathBuf>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RecoveryDrillReport {
    pub backup_path: PathBuf,
    pub inventory_sha256: Option<[u8; 32]>,
    pub started_at: time::OffsetDateTime,
    pub completed_at: time::OffsetDateTime,
    pub elapsed_milliseconds: u64,
    pub restored_files: u64,
    pub restored_bytes: u64,
    pub success: bool,
    pub error: Option<String>,
    pub report_path: PathBuf,
}

pub struct BackupManager;

impl BackupManager {
    pub fn create(
        brain_home: impl AsRef<Path>,
        backup_root: impl AsRef<Path>,
        now: time::OffsetDateTime,
    ) -> Result<BackupReport> {
        let brain_home = fs::canonicalize(brain_home.as_ref())
            .with_context(|| format!("resolve brain home {}", brain_home.as_ref().display()))?;
        fs::create_dir_all(backup_root.as_ref())?;
        let backup_root = fs::canonicalize(backup_root.as_ref())?;
        ensure!(
            backup_root != brain_home,
            "backup root cannot be the brain home"
        );
        let backup_id = uuid::Uuid::now_v7();
        let staging = backup_root.join(format!(".staging-{backup_id}"));
        let published = backup_root.join(format!("{}-{backup_id}", now.unix_timestamp_nanos()));
        ensure!(
            !staging.exists() && !published.exists(),
            "backup ID collision"
        );
        fs::create_dir(&staging)?;

        let result = (|| {
            let source_files = collect_files(&brain_home, Some(&backup_root))?;
            let mut files = Vec::with_capacity(source_files.len());
            for source in source_files {
                let relative = source.strip_prefix(&brain_home)?.to_path_buf();
                validate_relative(&relative)?;
                let destination = staging.join(&relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let kind = classify(&relative);
                if kind == InventoryKind::Sqlite {
                    let connection = Connection::open(&source)?;
                    connection.backup(DatabaseName::Main, &destination, None)?;
                } else {
                    copy_synced(&source, &destination)?;
                }
                files.push(inventory_file(&staging, &relative, kind)?);
            }
            files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            let inventory_sha256 = inventory_hash(&files)?;
            let inventory = BackupInventory {
                format_version: BACKUP_FORMAT_VERSION,
                backup_id,
                created_at: now,
                binary_version: env!("CARGO_PKG_VERSION").to_owned(),
                files,
                inventory_sha256,
            };
            write_synced(
                &staging.join("backup.json"),
                &serde_json::to_vec_pretty(&inventory)?,
            )?;
            Self::verify_at(&staging, now)?;
            fs::rename(&staging, &published)?;
            Ok(BackupReport {
                backup_path: published,
                inventory,
            })
        })();
        if result.is_err() && staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    pub fn verify(backup_path: impl AsRef<Path>) -> Result<VerificationReport> {
        Self::verify_at(backup_path.as_ref(), time::OffsetDateTime::now_utc())
    }

    pub fn restore_isolated(
        backup_path: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<RestoreReport> {
        let backup_path = fs::canonicalize(backup_path.as_ref())?;
        let verification = Self::verify(&backup_path)?;
        let inventory = load_inventory(&backup_path)?;
        let destination = destination.as_ref();
        ensure!(!destination.exists(), "restore destination already exists");
        let parent = destination
            .parent()
            .context("restore destination has no parent")?;
        fs::create_dir_all(parent)?;
        let staging = parent.join(format!(".restore-{}", inventory.backup_id));
        ensure!(!staging.exists(), "restore staging path already exists");
        fs::create_dir(&staging)?;
        let result = (|| {
            for item in &inventory.files {
                validate_relative(&item.relative_path)?;
                let source = backup_path.join(&item.relative_path);
                let target = staging.join(&item.relative_path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                copy_synced(&source, &target)?;
            }
            verify_tree(&staging, &inventory)?;
            fs::rename(&staging, destination)?;
            Ok(RestoreReport {
                destination: fs::canonicalize(destination)?,
                inventory_sha256: inventory.inventory_sha256,
                restored_files: verification.file_count,
                restored_bytes: verification.total_bytes,
            })
        })();
        if result.is_err() && staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    pub fn apply_retention(
        backup_root: impl AsRef<Path>,
        policy: RetentionPolicy,
        dry_run: bool,
    ) -> Result<RetentionReport> {
        ensure!(
            policy.hourly > 0 && policy.daily > 0 && policy.monthly > 0,
            "backup retention counts must be positive"
        );
        let backup_root = fs::canonicalize(backup_root.as_ref())?;
        let mut points = Vec::new();
        let mut ignored = Vec::new();
        for entry in fs::read_dir(&backup_root)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                ignored.push(path);
                continue;
            }
            match load_inventory(&path) {
                Ok(inventory) => points.push((path, inventory.created_at)),
                Err(_) => ignored.push(path),
            }
        }
        points.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let retained_indexes = retained_indexes(&points, policy);
        let mut retained = Vec::new();
        let mut pruned = Vec::new();
        for (index, (path, _)) in points.into_iter().enumerate() {
            if retained_indexes.contains(&index) {
                retained.push(path);
            } else {
                pruned.push(path);
            }
        }
        ignored.sort();
        if !dry_run {
            for path in &pruned {
                let resolved = fs::canonicalize(path)?;
                ensure!(
                    resolved.parent() == Some(backup_root.as_path()),
                    "retention candidate escaped the backup root: {}",
                    resolved.display()
                );
                fs::remove_dir_all(&resolved)?;
            }
        }
        Ok(RetentionReport {
            backup_root,
            retained,
            pruned,
            ignored,
            dry_run,
        })
    }

    pub fn recovery_drill(
        backup_path: impl AsRef<Path>,
        drill_root: impl AsRef<Path>,
        now: time::OffsetDateTime,
    ) -> Result<RecoveryDrillReport> {
        let started = std::time::Instant::now();
        let backup_path = fs::canonicalize(backup_path.as_ref())?;
        fs::create_dir_all(drill_root.as_ref())?;
        let drill_root = fs::canonicalize(drill_root.as_ref())?;
        ensure!(
            !drill_root.starts_with(&backup_path),
            "drill root cannot be inside the selected backup"
        );
        let drill_id = uuid::Uuid::now_v7();
        let destination = drill_root.join(format!("restore-{drill_id}"));
        let reports = drill_root.join("reports");
        fs::create_dir_all(&reports)?;
        let report_path = reports.join(format!("{}-{drill_id}.json", now.unix_timestamp_nanos()));
        let result = Self::restore_isolated(&backup_path, &destination);
        let completed_at = time::OffsetDateTime::now_utc();
        let (inventory_sha256, restored_files, restored_bytes, success, error) = match result {
            Ok(restored) => {
                let resolved = fs::canonicalize(&restored.destination)?;
                ensure!(
                    resolved.parent() == Some(drill_root.as_path()),
                    "drill restore escaped its work root"
                );
                fs::remove_dir_all(&resolved)?;
                (
                    Some(restored.inventory_sha256),
                    restored.restored_files,
                    restored.restored_bytes,
                    true,
                    None,
                )
            }
            Err(error) => (None, 0, 0, false, Some(error.to_string())),
        };
        let report = RecoveryDrillReport {
            backup_path,
            inventory_sha256,
            started_at: now,
            completed_at,
            elapsed_milliseconds: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            restored_files,
            restored_bytes,
            success,
            error,
            report_path: report_path.clone(),
        };
        write_synced(&report_path, &serde_json::to_vec_pretty(&report)?)?;
        Ok(report)
    }

    fn verify_at(backup_path: &Path, now: time::OffsetDateTime) -> Result<VerificationReport> {
        let backup_path = fs::canonicalize(backup_path)?;
        let inventory = load_inventory(&backup_path)?;
        ensure!(
            inventory.format_version == BACKUP_FORMAT_VERSION,
            "unsupported backup format {}",
            inventory.format_version
        );
        verify_tree(&backup_path, &inventory)?;
        let mut sqlite_checks = 0_u64;
        for item in &inventory.files {
            if item.kind != InventoryKind::Sqlite {
                continue;
            }
            let connection = Connection::open_with_flags(
                backup_path.join(&item.relative_path),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let integrity: String =
                connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            ensure!(
                integrity == "ok",
                "SQLite integrity check failed: {integrity}"
            );
            if table_exists(&connection, "schema_migrations")? {
                let version: Option<i64> = connection.query_row(
                    "SELECT MAX(version) FROM schema_migrations",
                    [],
                    |row| row.get(0),
                )?;
                ensure!(
                    version.unwrap_or_default() <= i64::from(LEDGER_SCHEMA_VERSION),
                    "backup ledger schema is newer than this binary"
                );
            }
            sqlite_checks += 1;
        }
        Ok(VerificationReport {
            backup_path,
            file_count: u64::try_from(inventory.files.len())?,
            total_bytes: inventory.files.iter().map(|item| item.bytes).sum(),
            sqlite_integrity_checks: sqlite_checks,
            verified_at: now,
        })
    }
}

fn retained_indexes(
    points: &[(PathBuf, time::OffsetDateTime)],
    policy: RetentionPolicy,
) -> BTreeSet<usize> {
    let mut retained = BTreeSet::new();
    let mut hours = BTreeSet::new();
    let mut days = BTreeSet::new();
    let mut months = BTreeSet::new();
    for (index, (_, created_at)) in points.iter().enumerate() {
        let hour = created_at.unix_timestamp().div_euclid(3_600);
        let day = created_at.unix_timestamp().div_euclid(86_400);
        let month = (created_at.year(), created_at.month() as u8);
        if hours.len() < policy.hourly && hours.insert(hour) {
            retained.insert(index);
        }
        if days.len() < policy.daily && days.insert(day) {
            retained.insert(index);
        }
        if months.len() < policy.monthly && months.insert(month) {
            retained.insert(index);
        }
    }
    retained
}

fn collect_files(root: &Path, excluded_root: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "backup refuses symlink {}",
                path.display()
            );
            if excluded_root.is_some_and(|excluded| path.starts_with(excluded)) {
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() && !is_sqlite_sidecar_or_temporary(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_sqlite_sidecar_or_temporary(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.ends_with(".tmp")
        || name == "backup.json"
}

fn classify(path: &Path) -> InventoryKind {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let value = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if matches!(extension, "sqlite" | "db") {
        InventoryKind::Sqlite
    } else if value.ends_with(".jsonl.zst") || value.ends_with(".manifest.json") {
        InventoryKind::Segment
    } else if value.contains("/blobs/sha256/") && value.ends_with(".zst") {
        InventoryKind::Blob
    } else if matches!(extension, "json" | "toml" | "yaml" | "yml") {
        InventoryKind::Configuration
    } else if extension == "md" {
        InventoryKind::Projection
    } else {
        InventoryKind::Other
    }
}

fn inventory_file(root: &Path, relative: &Path, kind: InventoryKind) -> Result<InventoryFile> {
    let path = root.join(relative);
    Ok(InventoryFile {
        relative_path: relative.to_path_buf(),
        sha256: file_hash(&path)?,
        bytes: fs::metadata(path)?.len(),
        kind,
    })
}

fn inventory_hash(files: &[InventoryFile]) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(files)?).into())
}

fn load_inventory(root: &Path) -> Result<BackupInventory> {
    let inventory: BackupInventory = serde_json::from_slice(&fs::read(root.join("backup.json"))?)?;
    ensure!(
        inventory.inventory_sha256 == inventory_hash(&inventory.files)?,
        "backup inventory hash mismatch"
    );
    Ok(inventory)
}

fn verify_tree(root: &Path, inventory: &BackupInventory) -> Result<()> {
    let mut listed = BTreeSet::new();
    for item in &inventory.files {
        validate_relative(&item.relative_path)?;
        ensure!(
            listed.insert(item.relative_path.clone()),
            "duplicate backup inventory path"
        );
        let path = root.join(&item.relative_path);
        ensure!(
            path.is_file(),
            "backup file is missing: {}",
            item.relative_path.display()
        );
        ensure!(
            fs::metadata(&path)?.len() == item.bytes,
            "backup file size mismatch: {}",
            item.relative_path.display()
        );
        ensure!(
            file_hash(&path)? == item.sha256,
            "backup checksum mismatch: {}",
            item.relative_path.display()
        );
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty() && !path.is_absolute(),
        "backup path must be relative"
    );
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            bail!("backup path escapes its root: {}", path.display());
        }
    }
    Ok(())
}

fn copy_synced(source: &Path, destination: &Path) -> Result<()> {
    let mut input = OpenOptions::new().read(true).open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut output = OpenOptions::new().create_new(true).write(true).open(path)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    Ok(())
}

fn file_hash(path: &Path) -> Result<[u8; 32]> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )?)
}
