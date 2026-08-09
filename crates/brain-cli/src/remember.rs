//! Filing a conclusion back into the brain.
//!
//! Karpathy's second operation ends with a line this system had no answer to: *"good answers can be
//! filed back into the wiki as new pages… your explorations compound"*. Ours did not compound. A
//! query returned hits, you read them, and whatever you worked out went into chat history and
//! nowhere else — so the brain accumulated only what consolidation happened to notice, never what
//! anyone actually concluded.
//!
//! This is the missing primitive. It writes a memory the same way consolidation does, through the
//! same validation, with two deliberate differences:
//!
//! - **Evidence is given, not inferred.** You name the events the conclusion rests on. The ledger
//!   refuses the append if any of them is not there, so a filed claim is no less checkable than a
//!   distilled one — which is the whole reason this can exist at all.
//! - **Authority is `HumanCorrection`.** A conclusion someone deliberately filed outranks one a
//!   model distilled in passing, and the resolver already knows that ordering. So filing an answer
//!   is also how you correct the brain: the new claim wins on the same subject without deleting
//!   anything.

use anyhow::{Context, Result, ensure};
use brain_domain::{
    Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus, ProjectId, WorktreeId,
};
use brain_store::EventLedger;

#[derive(Debug, serde::Serialize)]
pub struct RememberedMemory {
    pub memory_id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub kind: String,
    pub title: String,
    pub evidence_ids: Vec<uuid::Uuid>,
    /// The versions this supersedes, if it was filed as a correction.
    pub supersedes: Vec<uuid::Uuid>,
}

pub struct RememberRequest<'a> {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub kind: MemoryKind,
    pub title: &'a str,
    pub content: &'a str,
    pub evidence_ids: Vec<uuid::Uuid>,
    /// Memory ids this conclusion replaces. Their current versions are superseded, not deleted.
    pub supersedes: Vec<uuid::Uuid>,
    pub now: time::OffsetDateTime,
}

/// How many events a derived citation set may cite.
///
/// Small on purpose. The point of a citation is that someone can check it; twenty of them is a
/// haystack wearing a receipt's clothes.
const DERIVED_EVIDENCE_LIMIT: usize = 4;

/// Find the turns that support a claim, so the person filing it does not have to.
///
/// **This is why the file-back loop had never once run.** `brain remember` shipped in June and
/// across 13,493 memories not one carries `human_correction` authority — every single one was
/// machine-derived. That was not apathy. The command demanded
/// `--evidence event:019fe4d2-c168-7882-...`, and nobody can know a UUID by hand. The interface
/// made the operation impossible, so the operation never happened, and the vault only ever
/// accumulated what consolidation noticed — never what anyone actually concluded.
///
/// The invariant is untouched: a filed claim still cites real events, and an uncited claim is still
/// refused. What changes is who does the finding. The claim's own text goes through `search()` —
/// the same BM25 + vector + graph fusion measured at 96.0% R@5 — and the top hits become the
/// citations. Those are the turns that actually discuss the claim, which is a stronger citation
/// than "the last few events of this session" would have been.
///
/// Returns empty when nothing matches, and the caller must refuse rather than invent: a conclusion
/// this corpus contains no trace of is exactly the claim that should not be filed silently.
pub fn derive_evidence(
    ledger: &EventLedger,
    project_id: ProjectId,
    claim: &str,
    session: Option<&str>,
) -> Result<Vec<uuid::Uuid>> {
    let mut query = brain_store::SearchQuery::text(project_id, claim)
        .with_limit(DERIVED_EVIDENCE_LIMIT * 4)
        .with_expansion();
    if let Some(session) = session {
        query = query.for_session(session.to_owned());
    }
    let hits = ledger.search(&query)?;
    Ok(hits
        .into_iter()
        // Events only. A memory citing another memory is a link, not evidence — evidence bottoms
        // out in something a person said or a tool did.
        .filter(|hit| hit.memory_id.is_none())
        .map(|hit| hit.source_id)
        .take(DERIVED_EVIDENCE_LIMIT)
        .collect())
}

pub fn remember(
    ledger: &mut EventLedger,
    request: RememberRequest<'_>,
) -> Result<RememberedMemory> {
    ensure!(!request.title.trim().is_empty(), "a memory needs a title");
    ensure!(
        !request.content.trim().is_empty(),
        "a memory needs content — a title alone is a label, not a claim"
    );
    // The rule that makes a filed memory as checkable as a distilled one. Without it this command
    // would be the one way to get an unsourced claim into a vault that has none.
    //
    // `derive_evidence` removes the burden of *finding* the citations without weakening this: the
    // ledger still refuses a claim that cites nothing, whether a person or a search chose them.
    ensure!(
        !request.evidence_ids.is_empty(),
        "a memory needs at least one `event:<uuid>` — an uncited claim is exactly what this brain \
         does not store. Omit --evidence to have it derived from the claim's own text; if that \
         found nothing, this corpus holds no trace of the claim and filing it would be a guess"
    );

    // Resolve what is being superseded *before* writing, so a bad id fails the whole thing rather
    // than leaving a new memory beside the old one it was meant to replace.
    let mut superseded_versions = Vec::new();
    for memory_id in &request.supersedes {
        let existing = ledger
            .current_memory(*memory_id)?
            .with_context(|| format!("no current memory {memory_id} to supersede"))?;
        superseded_versions.push(existing.version_id);
    }

    let record = MemoryRecord {
        id: uuid::Uuid::now_v7(),
        version_id: uuid::Uuid::now_v7(),
        scope: MemoryScope::Project(request.project_id),
        worktree_id: Some(request.worktree_id),
        task_id: None,
        kind: request.kind,
        title: request.title.trim().to_owned(),
        content: request.content.trim().to_owned(),
        valid_from: request.now,
        valid_to: None,
        recorded_at: request.now,
        confidence: 1.0,
        // Deliberately filed beats incidentally distilled, and the resolver already knows it.
        authority: Authority::HumanCorrection,
        evidence_ids: request.evidence_ids.clone(),
        supersedes: superseded_versions.clone(),
        status: MemoryStatus::Current,
    };
    ledger.append_memory(&record)?;

    Ok(RememberedMemory {
        memory_id: record.id,
        version_id: record.version_id,
        kind: record.kind.as_str().to_owned(),
        title: record.title,
        evidence_ids: request.evidence_ids,
        supersedes: superseded_versions,
    })
}
