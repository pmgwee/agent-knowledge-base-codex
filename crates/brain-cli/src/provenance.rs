//! Walking a claim back to the turns it came from.
//!
//! Every memory in this brain cites `event:<uuid>`, and until now that was a property of the data
//! with no way to exercise it. A citation nobody can follow is indistinguishable from a citation
//! nobody checked — the difference between a system that *is* auditable and one that merely says
//! it is, is whether there is a command.
//!
//! What it reports, and why each part is there:
//!
//! - **The claim**, as stored, with its kind, authority and status. A superseded memory answers a
//!   question differently from a current one, and reading the text alone will not tell you which.
//! - **Every cited event**, with the transcript file and byte offset it was captured from. That is
//!   the end of the chain this brain controls; past it lies a file on disk anyone can open.
//! - **Citations that no longer resolve**, called out rather than skipped. This turned out to be
//!   unreachable through the supported path — `append_memory` refuses a memory citing an event the
//!   ledger does not hold, so the guarantee is enforced at write time rather than checked at read
//!   time. The branch stays as defence in depth against corruption arriving some other way, and a
//!   test pins the write-time refusal that makes it dead code.

use anyhow::{Context, Result};
use brain_domain::ProjectId;
use brain_store::EventLedger;

#[derive(Debug, serde::Serialize)]
pub struct ProvenanceReport {
    pub memory_id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub authority: String,
    pub status: String,
    pub recorded_at: time::OffsetDateTime,
    pub valid_from: time::OffsetDateTime,
    pub valid_to: Option<time::OffsetDateTime>,
    pub supersedes: Vec<uuid::Uuid>,
    pub evidence: Vec<EvidenceTrace>,
    /// Cited events that are not in the ledger. Always empty on a healthy brain.
    pub unresolved: Vec<uuid::Uuid>,
    /// How many other versions this memory has had, this one included.
    pub version_count: usize,
}

impl ProvenanceReport {
    /// Whether every citation resolved.
    pub fn intact(&self) -> bool {
        self.unresolved.is_empty()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct EvidenceTrace {
    pub event_id: uuid::Uuid,
    pub event_type: String,
    pub harness: String,
    pub native_session_id: String,
    pub occurred_at: time::OffsetDateTime,
    /// The transcript this was captured from, and where in it.
    pub source_locator: String,
    pub source_offset: i64,
    pub git_branch: Option<String>,
    /// A short excerpt, so the report is readable without opening the transcript.
    pub excerpt: String,
}

/// Longest excerpt carried per event.
///
/// Enough to recognise a turn, not enough to reproduce it — the point of the report is to send you
/// to the source, not to replace it.
const EXCERPT_CHARACTERS: usize = 240;

pub fn verify_memory(
    ledger: &EventLedger,
    project_id: ProjectId,
    memory_id: uuid::Uuid,
) -> Result<ProvenanceReport> {
    let versions = ledger.memory_versions(memory_id)?;
    let record = versions
        .last()
        .cloned()
        .with_context(|| format!("no memory {memory_id} in this project"))?;

    let mut evidence = Vec::new();
    let mut unresolved = Vec::new();
    for event_id in &record.evidence_ids {
        match ledger.event(*event_id)? {
            Some(event) => evidence.push(EvidenceTrace {
                event_id: event.event_id,
                event_type: event.event_type.as_str().to_owned(),
                harness: event.harness.as_str().to_owned(),
                native_session_id: event.native_session_id,
                occurred_at: event.occurred_at,
                source_locator: event.source_locator,
                source_offset: event.source_offset,
                git_branch: event.git_branch,
                excerpt: excerpt(&event.payload),
            }),
            // Append-only evidence makes this impossible, which is why it is reported rather
            // than filtered away: if it ever happens, silence would be the worst outcome.
            None => unresolved.push(*event_id),
        }
    }
    // Oldest first, so reading the list follows the work in the order it happened.
    evidence.sort_by_key(|trace| trace.occurred_at);

    Ok(ProvenanceReport {
        memory_id: record.id,
        version_id: record.version_id,
        project_id,
        kind: record.kind.as_str().to_owned(),
        title: record.title,
        content: record.content,
        authority: format!("{:?}", record.authority),
        status: format!("{:?}", record.status),
        recorded_at: record.recorded_at,
        valid_from: record.valid_from,
        valid_to: record.valid_to,
        supersedes: record.supersedes,
        evidence,
        unresolved,
        version_count: versions.len(),
    })
}

fn excerpt(payload: &serde_json::Value) -> String {
    let text = ["content", "text", "summary", "message", "path", "file_path"]
        .iter()
        .find_map(|key| payload.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| payload.to_string());
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= EXCERPT_CHARACTERS {
        return text;
    }
    let head: String = text.chars().take(EXCERPT_CHARACTERS).collect();
    format!("{head}…")
}

/// Render the report for a terminal.
pub fn render(report: &ProvenanceReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("memory:{}\n", report.memory_id));
    out.push_str(&format!("  title      {}\n", report.title));
    out.push_str(&format!(
        "  kind       {}  ·  status {}  ·  authority {}\n",
        report.kind, report.status, report.authority
    ));
    out.push_str(&format!(
        "  version    {} (of {})\n",
        report.version_id, report.version_count
    ));
    out.push_str(&format!("  valid from {}\n", report.valid_from));
    if let Some(valid_to) = report.valid_to {
        out.push_str(&format!("  valid to   {valid_to}\n"));
    }
    if !report.supersedes.is_empty() {
        out.push_str(&format!(
            "  supersedes {} version(s)\n",
            report.supersedes.len()
        ));
    }
    out.push('\n');
    out.push_str(&format!("{}\n\n", report.content.trim()));

    out.push_str(&format!(
        "evidence — {} event(s), oldest first\n",
        report.evidence.len()
    ));
    for trace in &report.evidence {
        out.push_str(&format!(
            "\n  event:{}\n    {}  ·  {}  ·  session {}\n    {}\n    {}:{}\n    {}\n",
            trace.event_id,
            trace.occurred_at,
            trace.harness,
            trace.native_session_id,
            trace.git_branch.as_deref().map_or_else(
                || trace.event_type.clone(),
                |branch| format!("{}  ·  {branch}", trace.event_type)
            ),
            trace.source_locator,
            trace.source_offset,
            trace.excerpt,
        ));
    }
    if !report.unresolved.is_empty() {
        out.push_str(&format!(
            "\nUNRESOLVED — {} citation(s) point at events not in this ledger.\n\
             Evidence is append-only, so this should be impossible; treat it as corruption.\n",
            report.unresolved.len()
        ));
        for event_id in &report.unresolved {
            out.push_str(&format!("  event:{event_id}\n"));
        }
    }
    out
}
