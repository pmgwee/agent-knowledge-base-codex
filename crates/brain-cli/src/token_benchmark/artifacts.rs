use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_domain::ProjectId;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{BenchmarkSummary, GradeRecord, RunManifest, SampleRecord, TokenBenchmarkReport};

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RawArtifact {
    pub relative_path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
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
