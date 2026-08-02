use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use atomicwrites::{AllowOverwrite, AtomicFile};
use brain_domain::{MemoryRecord, MemoryScope, ProjectId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;

use crate::EventLedger;

const PROJECTION_SCHEMA_VERSION: u32 = 1;

pub struct MarkdownProjector {
    vault_root: PathBuf,
}

impl MarkdownProjector {
    pub fn new(vault_root: impl Into<PathBuf>) -> Self {
        Self {
            vault_root: vault_root.into(),
        }
    }

    pub fn rebuild_project(
        &self,
        ledger: &EventLedger,
        project_id: ProjectId,
    ) -> Result<ProjectionReport> {
        ensure!(
            ledger.project_id() == project_id,
            "project scope does not match projection ledger"
        );
        let project_root = project_vault_root(&self.vault_root, project_id);
        let generated_root = project_root.join("generated");
        std::fs::create_dir_all(project_root.join("notes"))?;
        std::fs::create_dir_all(generated_root.join("generations"))?;

        let mut rendered = ledger
            .current_project_memories()?
            .into_iter()
            .map(|memory| render_memory(project_id, memory))
            .collect::<Result<Vec<_>>>()?;
        rendered.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
        let generation = generation_id(&rendered);
        let generation_root = generated_root.join("generations").join(&generation);
        if generation_root.is_dir() {
            verify_rendered_generation(&generation_root, &rendered)?;
        } else {
            let staging = generated_root
                .join("generations")
                .join(format!("staging-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir(&staging)?;
            for item in &rendered {
                let path = staging.join(&item.relative_path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut file = std::fs::File::create(&path)?;
                file.write_all(item.content.as_bytes())?;
                file.sync_all()?;
            }
            verify_rendered_generation(&staging, &rendered)?;
            std::fs::rename(&staging, &generation_root).with_context(|| {
                format!("publish Markdown generation {}", generation_root.display())
            })?;
        }

        let manifest = ProjectionManifest {
            schema_version: PROJECTION_SCHEMA_VERSION,
            project_id,
            generation: generation.clone(),
            files: rendered
                .iter()
                .map(|item| ManifestEntry {
                    logical_path: slash_path(&item.logical_path),
                    relative_path: slash_path(&item.relative_path),
                    sha256: item.sha256.clone(),
                    memory_id: item.memory_id,
                    version_id: item.version_id,
                })
                .collect(),
        };
        let manifest_path = generated_root.join("current.json");
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        let _: ProjectionManifest = serde_json::from_slice(&bytes)?;
        AtomicFile::new(&manifest_path, AllowOverwrite)
            .write(|file| {
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .with_context(|| format!("publish projection manifest {}", manifest_path.display()))?;
        Ok(ProjectionReport {
            project_id,
            generation,
            generation_root,
            manifest_path,
            file_count: rendered.len(),
        })
    }

    pub fn verify_project(&self, project_id: ProjectId) -> Result<ProjectionVerification> {
        let manifest_path = project_vault_root(&self.vault_root, project_id)
            .join("generated")
            .join("current.json");
        let manifest: ProjectionManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).with_context(|| {
                format!("read projection manifest {}", manifest_path.display())
            })?)?;
        ensure!(
            manifest.schema_version == PROJECTION_SCHEMA_VERSION,
            "unsupported projection manifest schema {}",
            manifest.schema_version
        );
        ensure!(
            manifest.project_id == project_id,
            "projection manifest violates project scope"
        );
        let generation_root = manifest_path
            .parent()
            .expect("manifest has generated parent")
            .join("generations")
            .join(&manifest.generation);
        let mut errors = Vec::new();
        let mut files = Vec::new();
        let mut logical_paths = HashSet::new();
        for entry in &manifest.files {
            let relative = safe_relative_path(&entry.relative_path)?;
            if !logical_paths.insert(entry.logical_path.clone()) {
                errors.push(format!("duplicate logical path {}", entry.logical_path));
            }
            let path = generation_root.join(relative);
            files.push(path.clone());
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let actual = hex::encode(Sha256::digest(bytes));
                    if actual != entry.sha256 {
                        errors.push(format!("checksum mismatch for {}", entry.relative_path));
                    }
                }
                Err(_) => errors.push(format!("missing generated file {}", entry.relative_path)),
            }
        }
        Ok(ProjectionVerification {
            project_id,
            generation: manifest.generation,
            valid: errors.is_empty(),
            errors,
            files,
        })
    }
}

pub fn project_vault_root(vault_root: &Path, project_id: ProjectId) -> PathBuf {
    vault_root.join("projects").join(project_id.0.to_string())
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectionReport {
    pub project_id: ProjectId,
    pub generation: String,
    pub generation_root: PathBuf,
    pub manifest_path: PathBuf,
    pub file_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectionVerification {
    pub project_id: ProjectId,
    pub generation: String,
    pub valid: bool,
    pub errors: Vec<String>,
    pub files: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct ProjectionManifest {
    schema_version: u32,
    project_id: ProjectId,
    generation: String,
    files: Vec<ManifestEntry>,
}

#[derive(Serialize, Deserialize)]
struct ManifestEntry {
    logical_path: String,
    relative_path: String,
    sha256: String,
    memory_id: uuid::Uuid,
    version_id: uuid::Uuid,
}

struct RenderedMemory {
    logical_path: PathBuf,
    relative_path: PathBuf,
    content: String,
    sha256: String,
    memory_id: uuid::Uuid,
    version_id: uuid::Uuid,
}

fn render_memory(project_id: ProjectId, memory: MemoryRecord) -> Result<RenderedMemory> {
    ensure!(
        memory.scope == MemoryScope::Project(project_id),
        "memory violates Markdown project scope"
    );
    let logical_path = memory.projection_path();
    let prefix = PathBuf::from("projects")
        .join(project_id.0.to_string())
        .join("generated");
    let relative_path = logical_path
        .strip_prefix(&prefix)
        .context("memory projection path is outside its project generated root")?
        .to_path_buf();
    ensure_safe_relative(&relative_path)?;
    let evidence = memory
        .evidence_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let supersedes = memory
        .supersedes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let valid_from = memory.valid_from.format(&Rfc3339)?;
    let valid_to = memory
        .valid_to
        .map(|value| value.format(&Rfc3339))
        .transpose()?;
    let recorded_at = memory.recorded_at.format(&Rfc3339)?;
    let observation = memory
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let content = format!(
        concat!(
            "---\n",
            "title: {title}\n",
            "type: note\n",
            "permalink: {permalink}\n",
            "tags: [agent-brain, {kind}]\n",
            "brain_generated: true\n",
            "project_id: \"{project_id}\"\n",
            "memory_id: \"{memory_id}\"\n",
            "version_id: \"{version_id}\"\n",
            "kind: {kind}\n",
            "authority: {authority}\n",
            "status: {status}\n",
            "valid_from: \"{valid_from}\"\n",
            "valid_to: {valid_to}\n",
            "recorded_at: \"{recorded_at}\"\n",
            "confidence: {confidence}\n",
            "evidence_ids: {evidence}\n",
            "supersedes: {supersedes}\n",
            "---\n\n",
            "# {heading}\n\n",
            "## Observations\n\n",
            "- [{kind}] {observation}\n\n",
            "## Memory\n\n",
            "{body}\n\n",
            "## Evidence\n\n",
            "{evidence_lines}\n"
        ),
        title = yaml_string(&memory.title)?,
        permalink = yaml_string(&format!("brain-{}", memory.id))?,
        kind = memory.kind.as_str(),
        project_id = project_id.0,
        memory_id = memory.id,
        version_id = memory.version_id,
        authority = memory.authority.as_str(),
        status = memory.status.as_str(),
        valid_from = valid_from,
        valid_to = valid_to
            .map(|value| format!("\"{value}\""))
            .unwrap_or_else(|| "null".to_owned()),
        recorded_at = recorded_at,
        confidence = memory.confidence,
        evidence = serde_json::to_string(&evidence)?,
        supersedes = serde_json::to_string(&supersedes)?,
        heading = memory.title,
        observation = observation,
        body = memory.content,
        evidence_lines = evidence
            .iter()
            .map(|id| format!("- event:{id}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let sha256 = hex::encode(Sha256::digest(content.as_bytes()));
    Ok(RenderedMemory {
        logical_path,
        relative_path,
        content,
        sha256,
        memory_id: memory.id,
        version_id: memory.version_id,
    })
}

fn yaml_string(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn generation_id(rendered: &[RenderedMemory]) -> String {
    let mut digest = Sha256::new();
    for item in rendered {
        digest.update(slash_path(&item.logical_path));
        digest.update([0]);
        digest.update(&item.sha256);
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

fn verify_rendered_generation(root: &Path, rendered: &[RenderedMemory]) -> Result<()> {
    for item in rendered {
        let path = root.join(&item.relative_path);
        let actual =
            hex::encode(Sha256::digest(std::fs::read(&path).with_context(|| {
                format!("read staged projection {}", path.display())
            })?));
        ensure!(actual == item.sha256, "staged projection checksum mismatch");
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value.replace('/', std::path::MAIN_SEPARATOR_STR));
    ensure_safe_relative(&path)?;
    Ok(path)
}

fn ensure_safe_relative(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("projection manifest contains an unsafe relative path");
    }
    Ok(())
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
