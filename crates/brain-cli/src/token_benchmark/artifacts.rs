use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_domain::ProjectId;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::retrieval_benchmark::{RetrievalBenchmarkReport, RetrievalSplit};

use super::{BenchmarkSummary, GradeRecord, RunManifest, SampleRecord, TokenBenchmarkReport};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RawArtifact {
    pub relative_path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BenchmarkCommandRecord {
    pub record_id: Uuid,
    pub recorded_at: String,
    pub stage: String,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BenchmarkArtifacts {
    run_dir: PathBuf,
}

impl BenchmarkArtifacts {
    pub fn new(brain_home: impl AsRef<Path>, project_id: ProjectId, run_id: Uuid) -> Result<Self> {
        let root = brain_home
            .as_ref()
            .join("runtime")
            .join("token-benchmarks")
            .join(project_id.0.to_string());
        let run_dir = root.join(run_id.to_string());
        ensure!(
            run_dir.starts_with(&root),
            "benchmark run escaped project root"
        );
        Ok(Self { run_dir })
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.run_dir.join("manifest.json")
    }

    pub fn create_run(&self, manifest: &RunManifest) -> Result<()> {
        fs::create_dir_all(self.run_dir.join("raw"))
            .with_context(|| format!("create benchmark run {}", self.run_dir.display()))?;
        for name in [
            "samples.jsonl",
            "lifecycle.jsonl",
            "retrieval-cases.jsonl",
            "grades.jsonl",
            "commands.jsonl",
        ] {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.run_dir.join(name))?;
        }
        write_immutable_json(&self.manifest_path(), manifest)
    }

    pub fn manifest(&self) -> Result<RunManifest> {
        read_json(&self.manifest_path())
    }

    pub fn append_sample(&self, sample: &SampleRecord) -> Result<()> {
        let attempt_id = format!("{}:attempt:{}", sample.sample.sample_id, sample.attempt);
        append_immutable_jsonl(
            &self.run_dir.join("samples.jsonl"),
            &attempt_id,
            sample,
            |record: &SampleRecord| {
                format!("{}:attempt:{}", record.sample.sample_id, record.attempt)
            },
        )
    }

    pub fn samples(&self) -> Result<Vec<SampleRecord>> {
        read_jsonl(&self.run_dir.join("samples.jsonl"))
    }

    pub fn append_grade(&self, grade: &GradeRecord) -> Result<()> {
        append_immutable_jsonl(
            &self.run_dir.join("grades.jsonl"),
            &grade.opaque_id,
            grade,
            |record: &GradeRecord| record.opaque_id.clone(),
        )
    }

    pub fn append_command(&self, stage: &str, argv: Vec<String>) -> Result<BenchmarkCommandRecord> {
        ensure!(!stage.trim().is_empty(), "benchmark command stage is empty");
        ensure!(!argv.is_empty(), "benchmark command has no arguments");
        let record = BenchmarkCommandRecord {
            record_id: Uuid::now_v7(),
            recorded_at: time::OffsetDateTime::now_utc().format(&Rfc3339)?,
            stage: stage.to_owned(),
            argv,
        };
        append_immutable_jsonl(
            &self.run_dir.join("commands.jsonl"),
            &record.record_id.to_string(),
            &record,
            |record: &BenchmarkCommandRecord| record.record_id.to_string(),
        )?;
        self.write_checksums()?;
        Ok(record)
    }

    pub fn commands(&self) -> Result<Vec<BenchmarkCommandRecord>> {
        read_jsonl(&self.run_dir.join("commands.jsonl"))
    }

    pub fn append_lifecycle_record(
        &self,
        record_id: &str,
        kind: &str,
        value: &impl Serialize,
    ) -> Result<()> {
        let record = serde_json::json!({
            "record_id": record_id,
            "kind": kind,
            "value": value
        });
        append_immutable_jsonl(
            &self.run_dir.join("lifecycle.jsonl"),
            record_id,
            &record,
            |record: &serde_json::Value| {
                record
                    .get("record_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            },
        )
    }

    pub fn attach_retrieval_report(&self, report: &RetrievalBenchmarkReport) -> Result<()> {
        for result in &report.cases {
            append_immutable_jsonl(
                &self.run_dir.join("retrieval-cases.jsonl"),
                &result.id,
                result,
                |record| record.id.clone(),
            )?;
        }
        let split = report
            .cases
            .first()
            .map(|case| case.split)
            .context("retrieval report has no cases")?;
        let name = match split {
            RetrievalSplit::Calibration => "retrieval-calibration",
            RetrievalSplit::LockedTest => "retrieval-locked-test",
        };
        self.write_named_json(name, report)?;
        self.write_checksums()?;
        Ok(())
    }

    pub fn grades(&self) -> Result<Vec<GradeRecord>> {
        read_jsonl(&self.run_dir.join("grades.jsonl"))
    }

    pub fn write_raw(&self, sample_id: &str, kind: &str, bytes: &[u8]) -> Result<RawArtifact> {
        ensure!(
            sample_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character)),
            "unsafe sample id"
        );
        ensure!(
            kind.chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character)),
            "unsafe raw artifact kind"
        );
        let hash = sha256(bytes);
        let relative_path = PathBuf::from("raw").join(format!("{sample_id}.{kind}.{hash}.bin"));
        let path = self.run_dir.join(&relative_path);
        fs::create_dir_all(path.parent().expect("raw parent"))?;
        if path.exists() {
            ensure!(
                fs::read(&path)? == bytes,
                "content-addressed raw artifact changed"
            );
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(RawArtifact {
            relative_path,
            sha256: hash,
            bytes: bytes.len() as u64,
        })
    }

    pub fn write_report(&self, report: &TokenBenchmarkReport) -> Result<()> {
        write_json_replace(&self.run_dir.join("report.json"), report)
    }

    pub fn report(&self) -> Result<TokenBenchmarkReport> {
        read_json(&self.run_dir.join("report.json"))
    }

    pub fn write_summary(&self, summary: &BenchmarkSummary) -> Result<()> {
        write_json_replace(&self.run_dir.join("summary.json"), summary)
    }

    pub fn write_benchmark_markdown(&self, markdown: &str) -> Result<()> {
        let path = self.run_dir.join("BENCHMARK.md");
        AtomicFile::new(&path, AllowOverwrite).write(|file| file.write_all(markdown.as_bytes()))?;
        Ok(())
    }

    pub fn write_checksums(&self) -> Result<usize> {
        let mut files = Vec::new();
        collect_files(&self.run_dir, &self.run_dir, &mut files)?;
        files.sort_by(|left, right| left.0.cmp(&right.0));
        let mut output = String::new();
        for (relative, path) in &files {
            output.push_str(&sha256(&fs::read(path)?));
            output.push_str("  ");
            output.push_str(&relative.replace('\\', "/"));
            output.push('\n');
        }
        AtomicFile::new(self.run_dir.join("checksums.sha256"), AllowOverwrite)
            .write(|file| file.write_all(output.as_bytes()))?;
        Ok(files.len())
    }

    pub fn write_named_json<T: Serialize>(&self, name: &str, value: &T) -> Result<PathBuf> {
        ensure!(
            name.chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character)),
            "unsafe artifact name"
        );
        let path = self.run_dir.join(format!("{name}.json"));
        write_json_replace(&path, value)?;
        Ok(path)
    }

    pub fn read_named_json<T: DeserializeOwned>(&self, name: &str) -> Result<T> {
        ensure!(
            name.chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character)),
            "unsafe artifact name"
        );
        read_json(&self.run_dir.join(format!("{name}.json")))
    }

    pub fn write_immutable_file(&self, relative: &Path, bytes: &[u8]) -> Result<PathBuf> {
        ensure!(
            !relative.is_absolute()
                && relative
                    .components()
                    .all(|component| !matches!(component, std::path::Component::ParentDir)),
            "artifact path escaped run directory"
        );
        let path = self.run_dir.join(relative);
        if path.is_file() {
            ensure!(
                fs::read(&path)? == bytes,
                "immutable artifact {} changed",
                path.display()
            );
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(path)
    }

    pub fn summary(&self) -> Result<BenchmarkSummary> {
        read_json(&self.run_dir.join("summary.json"))
    }

    pub fn retire(&self, reason: &str) -> Result<()> {
        ensure!(!reason.trim().is_empty(), "retirement reason is required");
        write_immutable_json(
            &self.run_dir.join("retired.json"),
            &serde_json::json!({ "reason": reason }),
        )
    }

    pub fn is_retired(&self) -> bool {
        self.run_dir.join("retired.json").is_file()
    }
}

fn collect_files(root: &Path, current: &Path, output: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(root, &entry.path(), output)?;
        } else if entry.file_type()?.is_file() && entry.file_name() != "checksums.sha256" {
            let relative = entry
                .path()
                .strip_prefix(root)
                .context("checksum path escaped run root")?
                .to_string_lossy()
                .to_string();
            output.push((relative, entry.path()));
        }
    }
    Ok(())
}

pub fn latest_summary(
    brain_home: &Path,
    project_id: ProjectId,
) -> Result<Option<BenchmarkSummary>> {
    let root = brain_home
        .join("runtime")
        .join("token-benchmarks")
        .join(project_id.0.to_string());
    if !root.is_dir() {
        return Ok(None);
    }
    let mut summaries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || entry.path().join("retired.json").is_file() {
            continue;
        }
        let path = entry.path().join("summary.json");
        if path.is_file() {
            summaries.push(read_json::<BenchmarkSummary>(&path)?);
        }
    }
    summaries.sort_by(|left, right| left.completed_at.cmp(&right.completed_at));
    Ok(summaries.pop())
}

fn append_immutable_jsonl<T, F>(path: &Path, id: &str, value: &T, record_id: F) -> Result<()>
where
    T: Serialize + DeserializeOwned + PartialEq,
    F: Fn(&T) -> String,
{
    for existing in read_jsonl::<T>(path)? {
        if record_id(&existing) == id {
            if &existing == value {
                return Ok(());
            }
            bail!("conflicting append-only record for {id}");
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    BufReader::new(fs::File::open(path)?)
        .lines()
        .filter(|line| line.as_ref().map_or(true, |line| !line.trim().is_empty()))
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

fn write_immutable_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if path.is_file() {
        ensure!(
            fs::read(path)? == bytes,
            "immutable artifact {} changed",
            path.display()
        );
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_json_replace(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    AtomicFile::new(path, AllowOverwrite).write(|file| file.write_all(&bytes))?;
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("read {}", path.display()))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
