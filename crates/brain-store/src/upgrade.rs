use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::SUPPORTED_FORMATS;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::backup::rebuildable_exclusions;
use crate::migrations::{configure, migrate};
use crate::{BackupManager, SegmentManifest};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UpgradeIssue {
    pub path: PathBuf,
    pub message: String,
    pub blocking: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UpgradeReport {
    pub brain_home: PathBuf,
    pub compatible: bool,
    pub sqlite_databases: u64,
    pub segment_manifests: u64,
    pub raw_event_hash: [u8; 32],
    pub issues: Vec<UpgradeIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UpgradeStageReport {
    pub source: UpgradeReport,
    pub staged_brain_home: PathBuf,
    pub staged: UpgradeReport,
    pub raw_hashes_preserved: bool,
    pub requires_service_stop_for_cutover: bool,
}

pub struct UpgradeManager;

impl UpgradeManager {
    pub fn check(brain_home: impl AsRef<Path>) -> Result<UpgradeReport> {
        let brain_home = fs::canonicalize(brain_home.as_ref())?;
        // The raw-event hash below must aggregate exactly what a backup captures, so the
        // rebuildable exclusions apply here too — a frozen benchmark ledger under
        // runtime/token-benchmarks is scaffolding, and hashing it would make `stage`'s
        // comparison against the backup-restored copy fail on a healthy brain.
        let files = collect_files(&brain_home, &rebuildable_exclusions(&brain_home))?;
        let mut issues = Vec::new();
        let mut sqlite_databases = 0_u64;
        let mut segment_manifests = 0_u64;
        let mut raw_hashes = Vec::new();
        for path in files {
            if is_sqlite(&path) {
                sqlite_databases += 1;
                let connection =
                    Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                let integrity: String =
                    connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
                if integrity != "ok" {
                    issues.push(issue(
                        &brain_home,
                        &path,
                        format!("SQLite integrity: {integrity}"),
                    ));
                }
                if table_exists(&connection, "schema_migrations")? {
                    let version: Option<i64> = connection.query_row(
                        "SELECT MAX(version) FROM schema_migrations",
                        [],
                        |row| row.get(0),
                    )?;
                    if version.unwrap_or_default() > i64::from(SUPPORTED_FORMATS.ledger) {
                        issues.push(issue(
                            &brain_home,
                            &path,
                            format!(
                                "ledger schema {} is newer than supported {}",
                                version.unwrap_or_default(),
                                SUPPORTED_FORMATS.ledger
                            ),
                        ));
                    }
                    if table_exists(&connection, "events")? {
                        let mut statement = connection
                            .prepare("SELECT event_id, raw_hash FROM events ORDER BY event_id")?;
                        let rows = statement.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
                        })?;
                        for row in rows {
                            let (id, hash) = row?;
                            raw_hashes.push((path.clone(), id, hash));
                        }
                    }
                }
            } else if path.to_string_lossy().ends_with(".manifest.json") {
                segment_manifests += 1;
                match serde_json::from_slice::<SegmentManifest>(&fs::read(&path)?) {
                    Ok(manifest)
                        if manifest.format_version > SUPPORTED_FORMATS.segment_manifest =>
                    {
                        issues.push(issue(
                            &brain_home,
                            &path,
                            format!(
                                "segment format {} is newer than supported {}",
                                manifest.format_version, SUPPORTED_FORMATS.segment_manifest
                            ),
                        ));
                    }
                    Ok(_) => {}
                    Err(error) => issues.push(issue(
                        &brain_home,
                        &path,
                        format!("invalid segment manifest: {error}"),
                    )),
                }
            }
        }
        check_json_schema(
            &brain_home,
            &brain_home.join("projects.json"),
            SUPPORTED_FORMATS.project_registry,
            &mut issues,
        )?;
        check_json_schema(
            &brain_home,
            &brain_home.join("runtime/service.json"),
            SUPPORTED_FORMATS.service_config,
            &mut issues,
        )?;
        let raw_event_hash = aggregate_raw_hashes(&brain_home, &raw_hashes)?;
        Ok(UpgradeReport {
            brain_home,
            compatible: !issues.iter().any(|issue| issue.blocking),
            sqlite_databases,
            segment_manifests,
            raw_event_hash,
            issues,
        })
    }

    pub fn stage(
        brain_home: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        now: time::OffsetDateTime,
    ) -> Result<UpgradeStageReport> {
        let source = Self::check(brain_home.as_ref())?;
        ensure!(
            source.compatible,
            "source brain is not compatible with this binary"
        );
        let destination = destination.as_ref();
        ensure!(!destination.exists(), "upgrade destination already exists");
        let parent = destination
            .parent()
            .context("upgrade destination has no parent")?;
        fs::create_dir_all(parent)?;
        let canonical_parent = fs::canonicalize(parent)?;
        ensure!(
            !canonical_parent.starts_with(&source.brain_home),
            "staged upgrade must be outside the active brain"
        );
        let backup_root =
            canonical_parent.join(format!(".upgrade-backup-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&backup_root)?;
        let result = (|| {
            let backup = BackupManager::create(&source.brain_home, &backup_root, now)?;
            BackupManager::restore_isolated(&backup.backup_path, destination)?;
            for path in collect_files(destination, &rebuildable_exclusions(destination))? {
                if !is_sqlite(&path) {
                    continue;
                }
                let connection = Connection::open(&path)?;
                if table_exists(&connection, "schema_migrations")? {
                    configure(&connection)?;
                    migrate(&connection)?;
                    let integrity: String =
                        connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
                    ensure!(integrity == "ok", "staged SQLite integrity failed");
                }
            }
            let staged = Self::check(destination)?;
            ensure!(
                staged.compatible,
                "staged upgrade failed compatibility check"
            );
            ensure!(
                source.raw_event_hash == staged.raw_event_hash,
                "staged upgrade changed canonical raw event hashes"
            );
            Ok(UpgradeStageReport {
                source: source.clone(),
                staged_brain_home: staged.brain_home.clone(),
                staged,
                raw_hashes_preserved: true,
                requires_service_stop_for_cutover: true,
            })
        })();
        if backup_root.exists() {
            fs::remove_dir_all(&backup_root)?;
        }
        result
    }
}

fn collect_files(root: &Path, excluded: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if excluded.iter().any(|excluded| path.starts_with(excluded)) {
                continue;
            }
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn is_sqlite(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "sqlite" | "db"))
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )?)
}

fn issue(root: &Path, path: &Path, message: String) -> UpgradeIssue {
    UpgradeIssue {
        path: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
        message,
        blocking: true,
    }
}

fn check_json_schema(
    root: &Path,
    path: &Path,
    supported: u32,
    issues: &mut Vec<UpgradeIssue>,
) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if let Some(version) = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        && version > u64::from(supported)
    {
        issues.push(issue(
            root,
            path,
            format!("format version {version} is newer than supported {supported}"),
        ));
    }
    Ok(())
}

fn aggregate_raw_hashes(root: &Path, rows: &[(PathBuf, String, Vec<u8>)]) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    for (path, id, hash) in rows {
        digest.update(
            path.strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/")
                .as_bytes(),
        );
        digest.update([0]);
        digest.update(id.as_bytes());
        digest.update([0]);
        digest.update(hash);
        digest.update([0]);
    }
    Ok(digest.finalize().into())
}
