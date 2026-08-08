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

/// Bumped to 2 when index pages joined the manifest, making `memory_id` optional. The projector
/// rebuilds every few seconds, so a manifest at the old version is replaced rather than
/// migrated — and `verify_project` refusing to read one is the correct outcome in the interval.
const PROJECTION_SCHEMA_VERSION: u32 = 2;

/// Memories listed under one kind on the project index before it summarises the rest.
///
/// An index is for finding your way in, not for reading everything: past a screen or two of
/// links it stops being navigation. The total is always stated, so a truncated list never
/// implies the vault holds less than it does.
const MAX_INDEX_LINKS_PER_KIND: usize = 40;

/// How long a memory must be both old and unretrieved before the projection says so.
///
/// Both conditions together, and neither on its own means anything: a decision from March can be
/// perfectly current, and a memory recorded yesterday has had no chance to be wanted. Ninety days
/// is deliberately generous — this marks a note, and a mark that fires often is one readers learn
/// to skip.
const STALE_AFTER: time::Duration = time::Duration::days(90);

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

        let memories = ledger.current_project_memories()?;
        let links = LinkIndex::build(&memories);
        // No index for an empty projection. A vault with nothing in it should be empty, not hold
        // a single page announcing that — and `current.json` already records that the projector
        // ran.
        let index = (!memories.is_empty())
            .then(|| render_index(project_id, &memories, &links))
            .transpose()?;
        // Subject pages are what turn a pile of episodic notes into a wiki: one page per recurring
        // subject, gaining entries as work continues. Derived, never asserted — see `subjects`.
        let subject_pages = render_subject_pages(ledger, project_id, &memories, &links)?;
        // Derived, not stored: staleness is computed from age and access, both of which move on
        // their own, so a memory stops being stale the moment retrieval returns it. There is no
        // flag to clear and nothing to go out of date.
        let stale_ids = ledger
            .stale_memory_ids(time::OffsetDateTime::now_utc(), STALE_AFTER)
            .unwrap_or_default();
        let mut rendered = memories
            .into_iter()
            .map(|memory| {
                let stale = stale_ids.contains(&memory.id);
                render_memory(project_id, memory, &links, stale)
            })
            .collect::<Result<Vec<_>>>()?;
        rendered.extend(index);
        rendered.extend(subject_pages);
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
            publish_generation(&staging, &generation_root)?;
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
        let pruned =
            prune_superseded_generations(&generated_root.join("generations"), &generation)?;
        append_projection_log(&project_root, &generation, rendered.len())?;
        Ok(ProjectionReport {
            project_id,
            generation,
            generation_root,
            manifest_path,
            file_count: rendered.len(),
            pruned_generations: pruned,
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
    /// Superseded generations removed by this rebuild.
    pub pruned_generations: usize,
}

/// Move a verified staging directory into place, retrying briefly.
///
/// Windows refuses to rename a directory while anything holds a handle inside it, and something
/// usually does: an editor watching the vault, an indexer, a virus scanner. Obsidian in
/// particular begins reading notes the moment they appear, which is exactly the window this
/// rename lands in — the live vault logged repeated "publish Markdown generation" failures once
/// it was open, and every one of them abandoned that projection.
///
/// The handles are transient, so a few short retries clear almost all of them. Failing after
/// that is correct: the staged content is verified and discarded rather than published
/// half-moved, and the next tick rebuilds it.
fn publish_generation(staging: &Path, destination: &Path) -> Result<()> {
    const ATTEMPTS: u32 = 5;
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match std::fs::rename(staging, destination) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                // Another writer may have published this very generation while we staged it.
                // Identical content, so there is nothing left to do.
                if destination.is_dir() {
                    let _ = std::fs::remove_dir_all(staging);
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(
                    50 * u64::from(attempt + 1),
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(staging);
    Err(last
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("rename failed"))
        .context(format!(
            "publish Markdown generation {} after {ATTEMPTS} attempts",
            destination.display()
        )))
}

/// Append one line to the vault's chronological log.
///
/// The generated tree answers "what does this project know". It cannot answer "what happened, and
/// when" — a content-addressed generation replaces its predecessor, so the vault has no history of
/// its own even though the ledger underneath it is nothing but history.
///
/// The prefix is fixed on purpose. `## [date] kind | detail` makes the file greppable with no
/// parser at all, so `grep "^## \[" log.md | tail -5` is the whole query language. That property
/// is worth more here than any structure, because the reader is as likely to be a person in a
/// terminal as an agent with a Markdown parser.
///
/// A log that cannot be written is not a reason to fail a projection that already succeeded, so
/// this is best-effort by contract.
fn append_projection_log(project_root: &Path, generation: &str, file_count: usize) -> Result<()> {
    let path = project_root.join("log.md");
    let now = time::OffsetDateTime::now_utc();
    let line = format!(
        "## [{:04}-{:02}-{:02} {:02}:{:02}] projection | {file_count} notes | generation {}
",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        &generation[..12.min(generation.len())],
    );
    if !path.exists() {
        let header = "# Log

Append-only record of what this vault did and when.                       Grep it: `grep \"^## [\" log.md | tail -5`.

";
        let _ = std::fs::write(&path, header);
    }
    let mut file = match std::fs::OpenOptions::new().append(true).open(&path) {
        Ok(file) => file,
        Err(_) => return Ok(()),
    };
    let _ = file.write_all(line.as_bytes());
    Ok(())
}

/// Delete every generation the manifest no longer points at.
///
/// Generations are content-addressed, so a rebuild that changes anything publishes a new
/// directory and leaves the old one behind. Nothing ever collected them. On the live vault that
/// reached 110 stale generations holding 10,911 Markdown files against 558 current ones — so a
/// vault opened in Obsidian showed the same notes twenty times over, as disconnected islands,
/// and its graph was mostly duplicates.
///
/// Only the published generation survives. A superseded one is unreachable by construction:
/// `current.json` names exactly one, and any rebuild can recreate any of them from the ledger.
///
/// Staging directories are the second half of this problem and were the half left open. A pass
/// writes into `staging-<uuid>` and renames it into place, so a process that stops in between —
/// a deploy restarting the service, a machine sleeping — leaves the directory behind forever.
/// Skipping every `staging-*` was right for the one being written and wrong for all the others,
/// and the vault reached **5,834 orphaned files across two projects** that way, each one a
/// duplicate note in Obsidian's graph.
///
/// Age is what separates the two, because it is the only thing that can: a projection pass takes
/// seconds, so a staging directory older than [`ABANDONED_STAGING_AGE`] is not one anybody is
/// still writing.
///
/// The age comes from the directory's **name**, not from the filesystem. Staging directories are
/// named `staging-<uuid-v7>`, and a v7 UUID carries the millisecond it was minted, so the
/// directory already states when it was created — no mtime to be rewritten by a backup, a sync
/// client, or a restore. A name that does not parse is treated as *in use*: keeping a dead
/// directory another hour costs a little disk, and deleting a live one corrupts a publish.
fn prune_superseded_generations(generations_root: &Path, keep: &str) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(generations_root) else {
        return Ok(0);
    };
    let now = time::OffsetDateTime::now_utc();
    let mut pruned = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == keep {
            continue;
        }
        if let Some(staged) = name.strip_prefix("staging-")
            && !is_abandoned_staging(staged, now)
        {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && std::fs::remove_dir_all(entry.path()).is_ok()
        {
            pruned += 1;
        }
    }
    Ok(pruned)
}

/// How old a staging directory must be before it counts as abandoned.
///
/// A projection pass writes its files and renames within seconds, so an hour is three orders of
/// magnitude of headroom. Generous on purpose: the failure this guards against is deleting a
/// directory mid-publish, and disk is cheaper than that.
pub(crate) const ABANDONED_STAGING_AGE: time::Duration = time::Duration::hours(1);

pub(crate) fn is_abandoned_staging(staged: &str, now: time::OffsetDateTime) -> bool {
    let Ok(id) = uuid::Uuid::parse_str(staged) else {
        return false;
    };
    let Some(timestamp) = id.get_timestamp() else {
        return false;
    };
    let (seconds, nanos) = timestamp.to_unix();
    let Ok(created) = time::OffsetDateTime::from_unix_timestamp(seconds as i64) else {
        return false;
    };
    now - (created + time::Duration::nanoseconds(i64::from(nanos))) >= ABANDONED_STAGING_AGE
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
    /// Absent for generated index pages, which are navigation rather than memory. Optional so
    /// the manifest can describe both without a second file listing to keep in step.
    #[serde(default)]
    memory_id: Option<uuid::Uuid>,
    #[serde(default)]
    version_id: Option<uuid::Uuid>,
}

struct RenderedMemory {
    logical_path: PathBuf,
    relative_path: PathBuf,
    content: String,
    sha256: String,
    memory_id: Option<uuid::Uuid>,
    version_id: Option<uuid::Uuid>,
}

impl RenderedMemory {
    /// A generated page that is not a memory — an index, a map of contents.
    ///
    /// Carried through the same staging, checksum, and manifest path as memories so a vault is
    /// verified whole. An index that drifted from the notes it lists would be its own quiet
    /// failure, and the existing generation hash already catches exactly that.
    fn page(project_id: ProjectId, relative: &str, content: String) -> Result<Self> {
        let relative_path = safe_relative_path(relative)?;
        let logical_path = PathBuf::from("projects")
            .join(project_id.0.to_string())
            .join("generated")
            .join(&relative_path);
        let sha256 = hex::encode(Sha256::digest(content.as_bytes()));
        Ok(Self {
            logical_path,
            relative_path,
            content,
            sha256,
            memory_id: None,
            version_id: None,
        })
    }
}

/// Most related memories linked from one note.
///
/// A busy stretch of work can leave dozens of memories sharing evidence, and a note that links
/// to all of them says nothing about which matter. The cap is applied after sorting, so the
/// selection is deterministic and the generation hash stays stable.
const MAX_RELATED_LINKS: usize = 8;

/// Which memories link to which, derived from the ledger rather than proposed by a model.
///
/// Two relationships are represented, and both are facts already recorded:
///
/// - **Shared evidence.** Memories citing the same event were distilled from the same moment,
///   so a reader following one has reason to see the other. Every such edge is backed by an
///   `event:<uuid>` both notes already cite.
/// - **Supersession.** A memory that replaces another names it in `supersedes`.
///
/// Nothing here is inferred from prose. Obsidian's graph is only worth trusting if every edge
/// in it corresponds to something the ledger can prove.
struct LinkIndex {
    /// Memory id to (note stem, title). The stem is what a wikilink must name — Obsidian
    /// resolves `[[…]]` against filenames, so a link built from the id stopped resolving the
    /// moment notes were named after their titles.
    titles: std::collections::BTreeMap<uuid::Uuid, (String, String)>,
    related: std::collections::BTreeMap<uuid::Uuid, Vec<uuid::Uuid>>,
}

impl LinkIndex {
    fn build(memories: &[MemoryRecord]) -> Self {
        let titles = memories
            .iter()
            .map(|memory| {
                let file = memory.projection_file_name();
                let stem = file.trim_end_matches(".md").to_owned();
                (memory.id, (stem, memory.title.clone()))
            })
            .collect();

        // Group by evidence first: one pass over memories, then one pass per shared event.
        let mut by_event: std::collections::BTreeMap<uuid::Uuid, Vec<uuid::Uuid>> =
            std::collections::BTreeMap::new();
        for memory in memories {
            for evidence in &memory.evidence_ids {
                by_event.entry(*evidence).or_default().push(memory.id);
            }
        }

        let mut related: std::collections::BTreeMap<uuid::Uuid, std::collections::BTreeSet<_>> =
            std::collections::BTreeMap::new();
        for sharing in by_event.values() {
            for left in sharing {
                for right in sharing {
                    if left != right {
                        related.entry(*left).or_default().insert(*right);
                    }
                }
            }
        }

        Self {
            titles,
            related: related
                .into_iter()
                .map(|(id, peers)| (id, peers.into_iter().take(MAX_RELATED_LINKS).collect()))
                .collect(),
        }
    }

    /// An Obsidian wikilink to a memory, aliased to its title.
    ///
    /// The target is the memory id because that is the note's filename; the alias is the title
    /// so the graph reads as sentences rather than UUIDs. A link is only emitted for a memory
    /// that exists in this projection — a dangling link is worse than an absent one, because it
    /// invites a reader to look for a note that was never written.
    fn wikilink(&self, id: uuid::Uuid) -> Option<String> {
        let (stem, title) = self.titles.get(&id)?;
        Some(format!(
            "[[{stem}|{}]]",
            title.replace(['[', ']', '|'], " ")
        ))
    }

    fn related_links(&self, id: uuid::Uuid) -> Vec<String> {
        self.related
            .get(&id)
            .map(|peers| {
                peers
                    .iter()
                    .filter_map(|peer| self.wikilink(*peer))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn render_memory(
    project_id: ProjectId,
    memory: MemoryRecord,
    links: &LinkIndex,
    stale: bool,
) -> Result<RenderedMemory> {
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
            "stale: {stale}\n",
            "evidence_ids: {evidence}\n",
            "supersedes: {supersedes}\n",
            "---\n\n",
            "# {heading}\n\n",
            "## Observations\n\n",
            "- [{kind}] {observation}\n\n",
            "## Memory\n\n",
            "{body}\n\n",
            "{relations}",
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
        stale = stale,
        evidence = serde_json::to_string(&evidence)?,
        supersedes = serde_json::to_string(&supersedes)?,
        heading = memory.title,
        observation = observation,
        body = memory.content,
        relations = render_relations(&memory, links),
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
        memory_id: Some(memory.id),
        version_id: Some(memory.version_id),
    })
}

/// One page per recurring subject, each asserting nothing of its own.
///
/// The rule the project index already follows applies here with more force: everything on the page
/// is a title and a link to a note that carries its own evidence, so a subject page cannot become
/// wrong independently of the memories it lists. It compounds in *coverage* — a new memory
/// mentioning the subject appears at the next projection, with no revision step and nothing to go
/// stale.
///
/// Returns nothing when no model is installed, because subjects are derived from embeddings and a
/// brain without vectors has no way to tell `deploy` from `fix`. That is the honest outcome: fewer
/// pages, never invented ones.
fn render_subject_pages(
    ledger: &EventLedger,
    project_id: ProjectId,
    memories: &[MemoryRecord],
    links: &LinkIndex,
) -> Result<Vec<RenderedMemory>> {
    let vectors: std::collections::HashMap<uuid::Uuid, Vec<f32>> = ledger
        .current_memory_vectors()
        .unwrap_or_default()
        .into_iter()
        .collect();
    if vectors.is_empty() {
        return Ok(Vec::new());
    }
    let inputs: Vec<crate::SubjectInput<'_>> = memories
        .iter()
        .map(|memory| crate::SubjectInput {
            memory_id: memory.id,
            title: &memory.title,
        })
        .collect();
    let subjects = crate::derive_subjects(&inputs, &vectors);

    let titles: std::collections::HashMap<uuid::Uuid, &MemoryRecord> =
        memories.iter().map(|memory| (memory.id, memory)).collect();

    subjects
        .iter()
        .map(|subject| {
            let mut listed: Vec<&MemoryRecord> = subject
                .memory_ids
                .iter()
                .filter_map(|id| titles.get(id).copied())
                .collect();
            // Newest first: a subject page is read for what happened lately far more often than
            // for where it started. Ties break on id so the generation hash is stable.
            listed.sort_by(|left, right| {
                right
                    .valid_from
                    .cmp(&left.valid_from)
                    .then_with(|| left.id.cmp(&right.id))
            });

            let mut body = String::new();
            body.push_str(&format!(
                concat!(
                    "---
",
                    "title: \"{term}\"
",
                    "type: note
",
                    "permalink: {permalink}
",
                    "tags: [agent-brain, subject]
",
                    "brain_generated: true
",
                    "project_id: \"{project_id}\"
",
                    "memory_count: {count}
",
                    "---

",
                    "# {term}

",
                    "{count} memories mention this subject. Every entry links to a note that
",
                    "carries its own evidence citations; nothing is asserted here.

",
                    "This page exists because those memories sit measurably closer together than
",
                    "two memories picked at random from this project — {lift:.3} above the
",
                    "baseline. The subject was derived, not proposed.

"
                ),
                term = subject.term,
                permalink = yaml_string(&format!("brain-subject-{}", subject.term))?,
                project_id = project_id.0,
                count = listed.len(),
                lift = subject.lift,
            ));
            for memory in &listed {
                if let Some(link) = links.wikilink(memory.id) {
                    body.push_str(&format!("- {link}\n"));
                }
            }
            body.push('\n');
            RenderedMemory::page(
                project_id,
                &format!("subjects/{}.md", slug_for_subject(&subject.term)),
                body,
            )
        })
        .collect()
}

/// A filesystem-safe name for a subject page.
fn slug_for_subject(term: &str) -> String {
    term.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The project index — the note to open first.
///
/// Peer links alone make a graph you can only enter if you already know a note. This gives the
/// vault a front door: what the project knows, grouped by kind, most recent first, every entry
/// a link. Obsidian users would call it a map of contents.
///
/// It carries no claims of its own. Everything on it is a title and a link to a note that
/// states its own evidence, so the index cannot become wrong independently of the memories.
fn render_index(
    project_id: ProjectId,
    memories: &[MemoryRecord],
    links: &LinkIndex,
) -> Result<RenderedMemory> {
    let mut by_kind: std::collections::BTreeMap<&'static str, Vec<&MemoryRecord>> =
        std::collections::BTreeMap::new();
    for memory in memories {
        by_kind
            .entry(memory.kind.as_str())
            .or_default()
            .push(memory);
    }

    let mut body = String::new();
    body.push_str(&format!(
        concat!(
            "---\n",
            "title: \"Project memory index\"\n",
            "type: note\n",
            "permalink: {permalink}\n",
            "tags: [agent-brain, index]\n",
            "brain_generated: true\n",
            "project_id: \"{project_id}\"\n",
            "memory_count: {count}\n",
            "---\n\n",
            "# Project memory index\n\n",
            "{count} memories distilled from captured sessions. Every entry links to a note that\n",
            "carries its own evidence citations; nothing is asserted here.\n\n"
        ),
        permalink = yaml_string(&format!("brain-index-{}", project_id.0))?,
        project_id = project_id.0,
        count = memories.len(),
    ));

    for (kind, mut group) in by_kind {
        // Newest first: an index is read for what happened lately far more often than for what
        // happened first. Ties break on id so the ordering — and the generation hash — is stable.
        group.sort_by(|left, right| {
            right
                .valid_from
                .cmp(&left.valid_from)
                .then_with(|| left.id.cmp(&right.id))
        });
        body.push_str(&format!("## {kind} ({})\n\n", group.len()));
        for memory in group.iter().take(MAX_INDEX_LINKS_PER_KIND) {
            if let Some(link) = links.wikilink(memory.id) {
                body.push_str(&format!("- {link}\n"));
            }
        }
        if group.len() > MAX_INDEX_LINKS_PER_KIND {
            body.push_str(&format!(
                "- …and {} more, in `{kind}/`\n",
                group.len() - MAX_INDEX_LINKS_PER_KIND
            ));
        }
        body.push('\n');
    }

    RenderedMemory::page(project_id, "index.md", body)
}

/// The wikilink sections, or nothing at all when a memory stands alone.
///
/// An empty "Related" heading is worse than no heading: it reads as a claim that the memory was
/// checked for relatives and has none, when it usually means it is the only one citing its
/// evidence so far. Sections appear only when they have contents.
fn render_relations(memory: &MemoryRecord, links: &LinkIndex) -> String {
    let mut out = String::new();

    let superseded: Vec<String> = memory
        .supersedes
        .iter()
        .filter_map(|id| links.wikilink(*id))
        .collect();
    if !superseded.is_empty() {
        out.push_str("## Supersedes\n\n");
        for link in &superseded {
            out.push_str(&format!("- {link}\n"));
        }
        out.push('\n');
    }

    let related = links.related_links(memory.id);
    if !related.is_empty() {
        out.push_str("## Related\n\n");
        for link in &related {
            out.push_str(&format!("- {link}\n"));
        }
        out.push('\n');
    }

    out
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
