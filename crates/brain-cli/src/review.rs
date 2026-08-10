//! The ingest half of A9 — ruling on memories the brain wrote but nobody has read.
//!
//! `brain revise --review-sheet` gates the *rewrite* path. This gates the path that produces the
//! claims in the first place: consolidation batches 200 events to a provider and appends whatever
//! survives validation, unattended. On 10 August that produced merges which passed every mechanical
//! rule — provenance, brevity, evidence retention — and asserted a falsehood, because the claims
//! they rested on asserted it. **No validator catches that, and no validator design will: the rules
//! are about form and this is about truth.**
//!
//! The gate is a status, not a queue table. A memory written `proposed` fails `CURRENT_CLAIM`, so
//! the orientation, `search` and the Markdown projection all skip it — it exists, it is cited, and
//! nothing acts on it until a person says so. `MemoryStatus::Proposed` was in the model from the
//! beginning and nothing had ever written it.
//!
//! **Approval and rejection are both appends.** Evidence is append-only; a rejected memory is not
//! deleted, it gains a version marked `invalid`, so the ledger records that somebody looked and
//! said no. That distinction matters the next time consolidation proposes the same claim.

use anyhow::{Result, bail};
use brain_domain::{MemoryRecord, MemoryStatus};
use brain_store::EventLedger;

/// How much of a claim the listing shows before it becomes a wall of text.
///
/// A reviewer deciding on twenty claims reads titles and skims bodies; anyone who needs the whole
/// thing has `--json`, and the point of the excerpt is to make the queue scannable rather than to
/// be the artefact the decision rests on.
const EXCERPT_CHARACTERS: usize = 400;

#[derive(Clone, Debug, serde::Serialize)]
pub struct PendingMemory {
    pub id: uuid::Uuid,
    pub kind: String,
    pub title: String,
    pub excerpt: String,
    pub evidence: Vec<uuid::Uuid>,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ReviewReport {
    pub pending: Vec<PendingMemory>,
    /// Set when this invocation ruled on something.
    pub approved: usize,
    pub rejected: usize,
}

pub fn pending(ledger: &EventLedger) -> Result<Vec<PendingMemory>> {
    let mut out = Vec::new();
    for id in ledger.proposed_memory_ids()? {
        let Some(memory) = ledger.current_memory(id)? else {
            continue;
        };
        out.push(PendingMemory {
            id: memory.id,
            kind: memory.kind.as_str().to_owned(),
            title: memory.title.clone(),
            excerpt: memory.content.chars().take(EXCERPT_CHARACTERS).collect(),
            evidence: memory.evidence_ids.clone(),
            recorded_at: memory
                .recorded_at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Rule on one memory, by appending the ruling as a new version.
///
/// Refuses anything not currently `proposed`. Approving an already-current memory would append a
/// redundant version, and "approving" a superseded one would quietly bring a retired claim back —
/// the same resurrection the reviewed-merge path refuses, for the same reason.
pub fn rule(
    ledger: &mut EventLedger,
    memory_id: uuid::Uuid,
    approve: bool,
    now: time::OffsetDateTime,
) -> Result<MemoryRecord> {
    let Some(memory) = ledger.current_memory(memory_id)? else {
        bail!("no memory {memory_id} in this project");
    };
    if memory.status != MemoryStatus::Proposed {
        bail!(
            "memory {memory_id} is {}, not proposed — there is nothing awaiting a decision here",
            memory.status.as_str()
        );
    }
    let mut ruled = memory.clone();
    ruled.version_id = uuid::Uuid::now_v7();
    ruled.recorded_at = now;
    ruled.status = if approve {
        MemoryStatus::Current
    } else {
        MemoryStatus::Invalid
    };
    // The ruling supersedes nothing: it is a new version of *this* memory, not a claim about any
    // other one. Carrying the proposal's `supersedes` forward would re-assert an edge the ledger
    // already holds.
    ruled.supersedes = Vec::new();
    ledger.append_memory(&ruled)?;
    Ok(ruled)
}

pub fn render(report: &ReviewReport) -> String {
    let mut out = String::new();
    if report.approved > 0 || report.rejected > 0 {
        out.push_str(&format!(
            "  {} approved · {} rejected\n\n",
            report.approved, report.rejected
        ));
    }
    if report.pending.is_empty() {
        out.push_str("  Nothing is waiting on a decision.\n");
        return out;
    }
    out.push_str(&format!(
        "  {} memory(ies) awaiting review — invisible to retrieval until approved\n\n",
        report.pending.len()
    ));
    for memory in &report.pending {
        out.push_str(&format!(
            "  {}  [{}]  {}\n",
            &memory.id.to_string()[..8],
            memory.kind,
            memory.title
        ));
        let flat = memory.excerpt.replace('\n', " ");
        out.push_str(&format!(
            "      {}\n      {} evidence event(s) · recorded {}\n",
            flat.chars().take(140).collect::<String>(),
            memory.evidence.len(),
            memory.recorded_at
        ));
    }
    out.push_str(
        "\n  brain review --project <p> --approve <id>   # it becomes current\n  \
         brain review --project <p> --reject  <id>   # it is marked invalid, and kept\n",
    );
    out
}
