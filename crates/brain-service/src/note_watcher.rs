use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, GlobalPreferenceStore};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;

const MAX_NOTE_BYTES: u64 = 1_048_576;
const MAX_NOTES_PER_SCAN: usize = 10_000;

pub struct NoteWatcher {
    notes_root: PathBuf,
    project_id: ProjectId,
    worktree_id: WorktreeId,
}

pub struct GlobalPreferenceNoteWatcher {
    notes_root: PathBuf,
}

impl GlobalPreferenceNoteWatcher {
    pub fn new(notes_root: impl Into<PathBuf>) -> Self {
        Self {
            notes_root: notes_root.into(),
        }
    }

    pub fn scan_once(
        &self,
        store: &mut GlobalPreferenceStore,
        observed_at: time::OffsetDateTime,
    ) -> Result<NoteScanReport> {
        std::fs::create_dir_all(&self.notes_root)?;
        let mut report = NoteScanReport::default();
        for path in discover_notes(&self.notes_root)? {
            report.scanned += 1;
            let relative = slash_path(
                path.strip_prefix(&self.notes_root)
                    .context("discovered global note escaped notes root")?,
            );
            let bytes = std::fs::read(&path)?;
            let content_hash: [u8; 32] = Sha256::digest(&bytes).into();
            if u64::try_from(bytes.len())? > MAX_NOTE_BYTES {
                report.review_queued += usize::from(store.queue_note_review(
                    &relative,
                    content_hash,
                    "global preference note exceeds the one MiB limit",
                    observed_at,
                )?);
                continue;
            }
            if store.note_import_hash(&relative)? == Some(content_hash) {
                report.unchanged += 1;
                continue;
            }
            let parsed = std::str::from_utf8(&bytes)
                .context("global preference note is not valid UTF-8")
                .and_then(|text| parse_global_preference(text, observed_at));
            let note = match parsed {
                Ok(note) => note,
                Err(error) => {
                    report.review_queued += usize::from(store.queue_note_review(
                        &relative,
                        content_hash,
                        &error.to_string(),
                        observed_at,
                    )?);
                    continue;
                }
            };
            let preference_id = note
                .preference_id
                .unwrap_or_else(|| stable_uuid(&[b"global-preference", relative.as_bytes()]));
            let version_id = stable_uuid(&[
                b"global-preference-version",
                relative.as_bytes(),
                &content_hash,
            ]);
            let memory = MemoryRecord {
                id: preference_id,
                version_id,
                scope: MemoryScope::GlobalPreferences,
                worktree_id: None,
                task_id: None,
                kind: MemoryKind::Preference,
                title: note.title,
                content: note.content,
                valid_from: note.valid_from,
                valid_to: None,
                recorded_at: observed_at,
                confidence: 1.0,
                authority: Authority::HumanCorrection,
                evidence_ids: Vec::new(),
                supersedes: Vec::new(),
                status: MemoryStatus::Current,
            };
            match store.append_preference(&memory).and_then(|()| {
                store.record_note_promotion(
                    &relative,
                    content_hash,
                    preference_id,
                    version_id,
                    observed_at,
                )
            }) {
                Ok(()) => report.imported += 1,
                Err(error) => {
                    report.review_queued += usize::from(store.queue_note_review(
                        &relative,
                        content_hash,
                        &error.to_string(),
                        observed_at,
                    )?);
                }
            }
        }
        Ok(report)
    }
}

pub async fn run_notes_and_projections(
    config: crate::ServiceLaunchConfig,
    brain_home: PathBuf,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let (_, pressure) = tokio::sync::watch::channel(crate::DegradationState::default());
    run_notes_and_projections_with_pressure(config, brain_home, shutdown, pressure).await
}

pub async fn run_notes_and_projections_with_pressure(
    config: crate::ServiceLaunchConfig,
    brain_home: PathBuf,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    pressure: tokio::sync::watch::Receiver<crate::DegradationState>,
) -> Result<()> {
    let vault_root = brain_home.join("vault");
    let projector = brain_store::MarkdownProjector::new(&vault_root);
    let global_watcher =
        GlobalPreferenceNoteWatcher::new(vault_root.join("global-preferences").join("notes"));
    let global_store_path = brain_home
        .join("global-preferences")
        .join("preferences.sqlite");
    let mut projected_versions = HashMap::<ProjectId, u64>::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                if pressure.borrow().markdown_projection_paused {
                    continue;
                }
                match GlobalPreferenceStore::open(&global_store_path) {
                    Ok(mut store) => {
                        if let Err(error) = global_watcher.scan_once(&mut store, time::OffsetDateTime::now_utc()) {
                            tracing::warn!(error = %error, "global preference note scan degraded");
                        }
                    }
                    Err(error) => tracing::warn!(error = %error, "global preference store unavailable"),
                }
                for project in &config.projects {
                    let mut ledger = match EventLedger::open(&project.ledger_path, project.project_id) {
                        Ok(ledger) => ledger,
                        Err(error) => {
                            tracing::warn!(project_id = %project.project_id.0, error = %error, "note watcher could not open project ledger");
                            continue;
                        }
                    };
                    let notes_root = brain_store::project_vault_root(&vault_root, project.project_id)
                        .join("notes");
                    let watcher = NoteWatcher::new(
                        notes_root,
                        project.project_id,
                        project.worktree_id,
                    );
                    if let Err(error) = watcher.scan_once(&mut ledger, time::OffsetDateTime::now_utc()) {
                        tracing::warn!(project_id = %project.project_id.0, error = %error, "project note scan degraded");
                    }
                    let version_count = match ledger.memory_version_count() {
                        Ok(count) => count,
                        Err(error) => {
                            tracing::warn!(project_id = %project.project_id.0, error = %error, "projection version count unavailable");
                            continue;
                        }
                    };
                    if projected_versions.get(&project.project_id) != Some(&version_count) {
                        match projector.rebuild_project(&ledger, project.project_id) {
                            Ok(_) => {
                                projected_versions.insert(project.project_id, version_count);
                            }
                            Err(error) => {
                                tracing::warn!(project_id = %project.project_id.0, error = %error, "Markdown projection degraded");
                            }
                        }
                    }
                }
            }
        }
    }
}

impl NoteWatcher {
    pub fn new(
        notes_root: impl Into<PathBuf>,
        project_id: ProjectId,
        worktree_id: WorktreeId,
    ) -> Self {
        Self {
            notes_root: notes_root.into(),
            project_id,
            worktree_id,
        }
    }

    pub fn scan_once(
        &self,
        ledger: &mut EventLedger,
        observed_at: time::OffsetDateTime,
    ) -> Result<NoteScanReport> {
        ensure!(
            ledger.project_id() == self.project_id,
            "note watcher project does not match its ledger"
        );
        std::fs::create_dir_all(&self.notes_root)?;
        let mut report = NoteScanReport::default();
        for path in discover_notes(&self.notes_root)? {
            report.scanned += 1;
            let relative = slash_path(
                path.strip_prefix(&self.notes_root)
                    .context("discovered note escaped notes root")?,
            );
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.len() > MAX_NOTE_BYTES {
                let hash = review_hash(&relative, metadata.len());
                report.review_queued += usize::from(ledger.queue_note_review(
                    &relative,
                    hash,
                    "note exceeds the one MiB import limit",
                    observed_at,
                )?);
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let content_hash: [u8; 32] = Sha256::digest(&bytes).into();
            if ledger.note_import_hash(&relative)? == Some(content_hash) {
                report.unchanged += 1;
                continue;
            }
            let parsed = std::str::from_utf8(&bytes)
                .context("note is not valid UTF-8")
                .and_then(|text| parse_note(text, self.project_id, observed_at));
            let note = match parsed {
                Ok(note) => note,
                Err(error) => {
                    report.review_queued += usize::from(ledger.queue_note_review(
                        &relative,
                        content_hash,
                        &error.to_string(),
                        observed_at,
                    )?);
                    continue;
                }
            };
            if !ledger.evidence_belongs_to_project(&note.evidence_ids)? {
                report.review_queued += usize::from(ledger.queue_note_review(
                    &relative,
                    content_hash,
                    "one or more evidence IDs do not belong to this project",
                    observed_at,
                )?);
                continue;
            }
            match self.import_note(ledger, &relative, &bytes, content_hash, note, observed_at) {
                Ok(()) => report.imported += 1,
                Err(error) => {
                    report.review_queued += usize::from(ledger.queue_note_review(
                        &relative,
                        content_hash,
                        &error.to_string(),
                        observed_at,
                    )?);
                }
            }
        }
        Ok(report)
    }

    fn import_note(
        &self,
        ledger: &mut EventLedger,
        relative: &str,
        bytes: &[u8],
        content_hash: [u8; 32],
        note: HumanNote,
        observed_at: time::OffsetDateTime,
    ) -> Result<()> {
        let memory_id = note.memory_id.unwrap_or_else(|| {
            stable_uuid(&[
                b"obsidian-memory",
                self.project_id.0.as_bytes(),
                relative.as_bytes(),
            ])
        });
        let current = ledger.current_memory(memory_id)?;
        if let Some(current) = &current {
            ensure!(
                current.kind == note.kind,
                "a correction cannot change the memory kind"
            );
        }
        let version_id = stable_uuid(&[
            b"obsidian-version",
            self.project_id.0.as_bytes(),
            relative.as_bytes(),
            &content_hash,
        ]);
        let event_id = stable_uuid(&[
            b"obsidian-audit",
            self.project_id.0.as_bytes(),
            relative.as_bytes(),
            &content_hash,
        ]);
        let idempotency_key: [u8; 32] = hash_parts(&[
            b"obsidian-audit",
            self.project_id.0.as_bytes(),
            relative.as_bytes(),
            &content_hash,
        ]);
        let text = std::str::from_utf8(bytes)?;
        let payload = serde_json::json!({
            "content": note.content,
            "title": note.title,
            "kind": note.kind.as_str(),
            "memory_id": memory_id,
            "path": relative,
            "human_correction": true
        });
        ledger.append_batch(&EventBatch {
            source_id: format!("obsidian-note:{}:{relative}", self.project_id.0),
            events: vec![NormalizedEvent {
                event_id,
                project_id: self.project_id,
                worktree_id: self.worktree_id,
                task_id: None,
                harness: Harness::Other("obsidian".to_owned()),
                native_session_id: format!("obsidian:{relative}"),
                native_turn_id: None,
                event_type: EventType::CheckpointAuthored,
                occurred_at: note.valid_from,
                observed_at,
                source_locator: format!("vault:notes/{relative}"),
                source_offset: i64::try_from(bytes.len())?,
                source_schema: "obsidian-note:v1".to_owned(),
                raw_hash: content_hash,
                idempotency_key,
                git_head: None,
                git_branch: None,
                payload,
                raw: serde_json::json!({"markdown": text}),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::for_file(
                u64::try_from(bytes.len())?,
                hex::encode(content_hash),
            ),
        })?;
        let mut evidence_ids = note.evidence_ids;
        evidence_ids.push(event_id);
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        let memory = MemoryRecord {
            id: memory_id,
            version_id,
            scope: MemoryScope::Project(self.project_id),
            worktree_id: Some(self.worktree_id),
            task_id: None,
            kind: note.kind,
            title: note.title,
            content: note.content,
            valid_from: note.valid_from,
            valid_to: None,
            recorded_at: observed_at,
            confidence: 1.0,
            authority: Authority::HumanCorrection,
            evidence_ids,
            supersedes: current
                .map(|memory| vec![memory.version_id])
                .unwrap_or_default(),
            status: MemoryStatus::Current,
        };
        ledger.append_memory(&memory)?;
        ledger.record_note_import(
            relative,
            content_hash,
            memory.id,
            memory.version_id,
            observed_at,
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoteScanReport {
    pub scanned: usize,
    pub unchanged: usize,
    pub imported: usize,
    pub review_queued: usize,
}

struct HumanNote {
    title: String,
    kind: MemoryKind,
    memory_id: Option<uuid::Uuid>,
    evidence_ids: Vec<uuid::Uuid>,
    valid_from: time::OffsetDateTime,
    content: String,
}

struct GlobalPreferenceNote {
    title: String,
    preference_id: Option<uuid::Uuid>,
    valid_from: time::OffsetDateTime,
    content: String,
}

fn parse_note(
    text: &str,
    expected_project: ProjectId,
    observed_at: time::OffsetDateTime,
) -> Result<HumanNote> {
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    ensure!(
        lines.next() == Some("---"),
        "note must start with YAML front matter"
    );
    let mut fields = HashMap::new();
    let mut closed = false;
    for line in &mut lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .context("front matter entries must use key: value")?;
        ensure!(
            fields
                .insert(key.trim().to_owned(), value.trim().to_owned())
                .is_none(),
            "duplicate front matter key {}",
            key.trim()
        );
    }
    ensure!(closed, "note front matter is not closed");
    ensure!(
        fields.get("brain_generated").map(String::as_str) != Some("true"),
        "generated notes cannot be imported"
    );
    ensure!(
        fields.get("promote_global").map(String::as_str) != Some("true"),
        "global promotion is allowed only in the global-preferences notes root"
    );
    let project = uuid::Uuid::parse_str(&plain_string(required(&fields, "project_id")?))?;
    ensure!(
        project == expected_project.0,
        "note project ID does not match this notes root"
    );
    let kind_name = plain_string(required(&fields, "kind")?);
    let kind = MemoryKind::from_name(&kind_name).context("unsupported memory kind")?;
    ensure!(
        kind != MemoryKind::Preference,
        "project notes cannot create global preferences"
    );
    let title = plain_string(required(&fields, "title")?);
    ensure!(
        !title.trim().is_empty() && title.chars().count() <= 300,
        "invalid note title"
    );
    let memory_id = fields
        .get("memory_id")
        .map(|value| uuid::Uuid::parse_str(&plain_string(value)))
        .transpose()?;
    let evidence_ids = fields
        .get("evidence_ids")
        .map(|value| parse_uuid_list(value))
        .transpose()?
        .unwrap_or_default();
    let valid_from = fields
        .get("valid_from")
        .map(|value| time::OffsetDateTime::parse(&plain_string(value), &Rfc3339))
        .transpose()?
        .unwrap_or(observed_at);
    let content = lines.collect::<Vec<_>>().join("\n").trim().to_owned();
    ensure!(!content.is_empty(), "note body cannot be empty");
    ensure!(content.chars().count() <= 20_000, "note body is too large");
    Ok(HumanNote {
        title,
        kind,
        memory_id,
        evidence_ids,
        valid_from,
        content,
    })
}

fn parse_global_preference(
    text: &str,
    observed_at: time::OffsetDateTime,
) -> Result<GlobalPreferenceNote> {
    let (fields, content) = parse_frontmatter(text)?;
    ensure!(
        fields.get("promote_global").map(String::as_str) == Some("true"),
        "global preference notes require promote_global: true"
    );
    ensure!(
        !fields.contains_key("project_id"),
        "global preference notes cannot declare a project ID"
    );
    ensure!(
        plain_string(required(&fields, "kind")?) == "preference",
        "global notes accept only the preference kind"
    );
    let title = plain_string(required(&fields, "title")?);
    ensure!(
        !title.trim().is_empty() && title.chars().count() <= 300,
        "invalid preference title"
    );
    let preference_id = fields
        .get("preference_id")
        .map(|value| uuid::Uuid::parse_str(&plain_string(value)))
        .transpose()?;
    let valid_from = fields
        .get("valid_from")
        .map(|value| time::OffsetDateTime::parse(&plain_string(value), &Rfc3339))
        .transpose()?
        .unwrap_or(observed_at);
    ensure!(!content.is_empty(), "preference body cannot be empty");
    ensure!(
        content.chars().count() <= 20_000,
        "preference body is too large"
    );
    Ok(GlobalPreferenceNote {
        title,
        preference_id,
        valid_from,
        content,
    })
}

fn parse_frontmatter(text: &str) -> Result<(HashMap<String, String>, String)> {
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    ensure!(
        lines.next() == Some("---"),
        "note must start with YAML front matter"
    );
    let mut fields = HashMap::new();
    let mut closed = false;
    for line in &mut lines {
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .context("front matter entries must use key: value")?;
        ensure!(
            fields
                .insert(key.trim().to_owned(), value.trim().to_owned())
                .is_none(),
            "duplicate front matter key {}",
            key.trim()
        );
    }
    ensure!(closed, "note front matter is not closed");
    Ok((
        fields,
        lines.collect::<Vec<_>>().join("\n").trim().to_owned(),
    ))
}

fn required<'a>(fields: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    fields
        .get(key)
        .map(String::as_str)
        .with_context(|| format!("missing required front matter key {key}"))
}

fn plain_string(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim().to_owned())
}

fn parse_uuid_list(value: &str) -> Result<Vec<uuid::Uuid>> {
    let value = value.trim();
    ensure!(
        value.starts_with('[') && value.ends_with(']'),
        "evidence_ids must be a list"
    );
    let body = &value[1..value.len() - 1];
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    body.split(',')
        .map(|item| uuid::Uuid::parse_str(&plain_string(item.trim())).map_err(Into::into))
        .collect()
}

fn discover_notes(root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut notes = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            {
                notes.push(path);
                ensure!(
                    notes.len() <= MAX_NOTES_PER_SCAN,
                    "notes root exceeds scan limit"
                );
            }
        }
    }
    notes.sort();
    Ok(notes)
}

fn stable_uuid(parts: &[&[u8]]) -> uuid::Uuid {
    let hash = hash_parts(parts);
    let mut bytes: [u8; 16] = hash[..16].try_into().expect("slice is 16 bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

fn hash_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
        digest.update([0]);
    }
    digest.finalize().into()
}

fn review_hash(path: &str, size: u64) -> [u8; 32] {
    hash_parts(&[b"oversized-note", path.as_bytes(), &size.to_le_bytes()])
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
