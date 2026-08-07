//! Getting your data out.
//!
//! The brain holds every keystroke of your work in a SQLite file it owns, and until now the only
//! ways to read it were the brain's own commands or a SQLite client. That is fine while the brain
//! works and unacceptable as a permanent arrangement: a memory system you cannot get data out of
//! is a memory system you cannot leave, and one whose contents you cannot check with anything
//! except the thing being checked.
//!
//! Two formats, because they answer different questions:
//!
//! - **JSONL**, one record per line, machine-readable and streamable. Nothing is nested and nothing
//!   is pretty-printed, so a 100k-event export can be `grep`ed, split, or piped without a parser
//!   holding it all in memory.
//! - **Markdown**, one file per memory plus an index. The same shape the vault already uses, so an
//!   export opens in Obsidian without conversion.
//!
//! **Evidence is exported with its memories, not separately.** A memory without the turns it cites
//! is exactly the unsourced claim this system exists to avoid, and an export that dropped the
//! citations would produce one — quietly, at the moment the data leaves the only place that could
//! have resolved them.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_domain::ProjectId;
use brain_store::EventLedger;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum ExportFormat {
    /// One JSON record per line.
    Jsonl,
    /// One Markdown file per memory, plus an index.
    Markdown,
    /// Both.
    Both,
}

#[derive(Debug, serde::Serialize)]
pub struct ExportReport {
    pub project_id: ProjectId,
    pub destination: PathBuf,
    pub memories: usize,
    pub events: usize,
    pub bytes: u64,
    pub files: Vec<PathBuf>,
    /// Memories whose evidence could not be resolved. Always empty on a healthy brain.
    pub memories_missing_evidence: usize,
}

/// Export a project.
///
/// `since` bounds by when a memory became valid, not by when it was recorded, because the question
/// people ask is "what did I decide after X" rather than "what did the consolidator get round to".
pub fn export_project(
    ledger: &EventLedger,
    project_id: ProjectId,
    destination: &Path,
    format: ExportFormat,
    since: Option<time::OffsetDateTime>,
) -> Result<ExportReport> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("create export directory {}", destination.display()))?;

    let memories = ledger
        .current_project_memories()?
        .into_iter()
        .filter(|memory| since.is_none_or(|since| memory.valid_from >= since))
        .collect::<Vec<_>>();

    let mut files = Vec::new();
    let mut event_count = 0;
    let mut missing_evidence = 0;

    if matches!(format, ExportFormat::Jsonl | ExportFormat::Both) {
        let (written, events, absent) = write_jsonl(ledger, destination, &memories)?;
        files.extend(written);
        event_count = events;
        missing_evidence = absent;
    }
    if matches!(format, ExportFormat::Markdown | ExportFormat::Both) {
        files.extend(write_markdown(destination, &memories)?);
    }

    let bytes = files
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();

    Ok(ExportReport {
        project_id,
        destination: destination.to_path_buf(),
        memories: memories.len(),
        events: event_count,
        bytes,
        files,
        memories_missing_evidence: missing_evidence,
    })
}

fn write_jsonl(
    ledger: &EventLedger,
    destination: &Path,
    memories: &[brain_domain::MemoryRecord],
) -> Result<(Vec<PathBuf>, usize, usize)> {
    let memories_path = destination.join("memories.jsonl");
    let events_path = destination.join("events.jsonl");
    let mut memory_file = std::io::BufWriter::new(std::fs::File::create(&memories_path)?);
    let mut event_file = std::io::BufWriter::new(std::fs::File::create(&events_path)?);

    // Every cited event is exported exactly once however many memories cite it, so the file is
    // the evidence set rather than a per-memory transcript with duplicates.
    let mut seen = std::collections::HashSet::new();
    let mut missing = 0;
    for memory in memories {
        serde_json::to_writer(&mut memory_file, &memory)?;
        memory_file.write_all(b"\n")?;

        let mut resolved_all = true;
        for event_id in &memory.evidence_ids {
            if !seen.insert(*event_id) {
                continue;
            }
            match ledger.event(*event_id)? {
                Some(event) => {
                    serde_json::to_writer(&mut event_file, &event)?;
                    event_file.write_all(b"\n")?;
                }
                None => resolved_all = false,
            }
        }
        if !resolved_all {
            missing += 1;
        }
    }
    memory_file.flush()?;
    event_file.flush()?;
    Ok((vec![memories_path, events_path], seen.len(), missing))
}

fn write_markdown(
    destination: &Path,
    memories: &[brain_domain::MemoryRecord],
) -> Result<Vec<PathBuf>> {
    let notes = destination.join("memories");
    std::fs::create_dir_all(&notes)?;
    let mut files = Vec::new();

    for memory in memories {
        let path = notes.join(memory.projection_file_name());
        let mut body = String::new();
        body.push_str("---\n");
        body.push_str(&format!("title: {:?}\n", memory.title));
        body.push_str(&format!("kind: {}\n", memory.kind.as_str()));
        body.push_str(&format!("memory_id: {}\n", memory.id));
        body.push_str(&format!("valid_from: {}\n", memory.valid_from));
        body.push_str(&format!("confidence: {}\n", memory.confidence));
        body.push_str("---\n\n");
        body.push_str(&format!("# {}\n\n{}\n\n", memory.title, memory.content));
        body.push_str("## Evidence\n\n");
        for event_id in &memory.evidence_ids {
            body.push_str(&format!("- `event:{event_id}`\n"));
        }
        std::fs::write(&path, body)?;
        files.push(path);
    }

    let mut index = String::from("# Exported memories\n\n");
    index.push_str(&format!("{} memories.\n\n", memories.len()));
    for memory in memories {
        index.push_str(&format!(
            "- [{}](memories/{})\n",
            memory.title,
            memory.projection_file_name()
        ));
    }
    let index_path = destination.join("index.md");
    std::fs::write(&index_path, index)?;
    files.push(index_path);
    Ok(files)
}
