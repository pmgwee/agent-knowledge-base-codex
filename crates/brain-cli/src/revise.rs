//! Finding the claims a new observation should have updated.
//!
//! The last operation of Karpathy's pattern this system does not perform, and the one his whole
//! argument rests on: *"a single source might touch 10–15 wiki pages."* We touch zero. A new
//! observation becomes a new memory beside the old ones, and nothing already written changes.
//!
//! Three things are easy to confuse, and only the third is missing:
//!
//! | | What it does | State |
//! |---|---|---|
//! | The fold (`reconcile --apply`) | Two claims that are the *same claim* merge | shipped — 97 folded |
//! | Synthesis prose (3.2b) | A paragraph above a subject page's list | quota-bound |
//! | **Cross-claim revision** | A newer claim makes an **older, different** claim incomplete | **this** |
//!
//! **Detection needs no provider, and that is the point.** A revision candidate is derivable from
//! rows the ledger already holds: two claims on the same subject, *not* contradicting (the fold
//! handles those), where the newer one rests on evidence the older one never saw. The older claim is
//! not wrong — it is **stale in a way staleness cannot detect**, because it was written before the
//! thing that completes it existed.
//!
//! What this deliberately does **not** do is rewrite anything. Merging two claims' meanings is a
//! judgement about language, and a model doing it silently is how a verifiable system becomes a
//! plausible one. This reports pairs and stops, the way `brain lint` did before `reconcile` existed
//! — and if the pairs turn out to be noise, the answer is stricter detection, not a louder feature.

use anyhow::Result;
use brain_domain::{MemoryRecord, ProjectId};
use brain_store::EventLedger;

/// How much newer evidence a claim needs before it is worth reviewing an older one against it.
///
/// One shared subject and one new event is too little — consolidation produces near-neighbours
/// constantly, and a candidate list nobody finishes reading is the same as no candidate list.
const MIN_NEW_EVIDENCE: usize = 2;

/// How far apart two claims must be before a revision is worth reviewing, by default.
///
/// Measured on this corpus: of 454 linked pairs, **441 were written the same day** — consolidation
/// windows overlapping, one episode described twice within an hour, which is the fold's business
/// rather than a revision. The 13 that span a day or more are the ones where something was *learnt
/// later*, and those are the ones worth a person's time. `--all` shows everything.
const DEFAULT_MIN_GAP_DAYS: f64 = 1.0;

/// A newer claim that appears to complete an older one on the same subject.
#[derive(Debug, serde::Serialize)]
pub struct RevisionCandidate {
    pub subject: String,
    pub older_id: uuid::Uuid,
    pub older_title: String,
    pub older_valid_from: time::OffsetDateTime,
    pub newer_id: uuid::Uuid,
    pub newer_title: String,
    pub newer_valid_from: time::OffsetDateTime,
    /// Events the newer claim cites that the older one never saw. This is the reason it is a
    /// candidate at all, and the thing a reviewer should read first.
    pub unseen_evidence: Vec<uuid::Uuid>,
    /// Days between them. A claim completed the same afternoon is a different thing from one
    /// completed three weeks later, and only a person can say which matters.
    pub gap_days: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct RevisionReport {
    pub project_id: ProjectId,
    pub subjects_examined: usize,
    pub candidates: Vec<RevisionCandidate>,
}

/// Every older claim a newer one on the same subject appears to complete.
///
/// **Grouped by the derived subject pages, not by title.** The first version of this used
/// `subject_key` — kind plus normalised title, the grouping `resolve_candidates` uses for
/// contradictions — and returned **zero candidates across 2,095 subjects**, because that key
/// requires two claims to carry the *identical* title and, after the fold, almost every one is a
/// singleton.
///
/// That grouping is correct for contradictions and wrong here. A contradiction is two statements of
/// the same claim; a revision is a *later* claim about the same **topic**. The 450 subject pages
/// already group by embedding tightness — measurably closer together than two memories drawn at
/// random — which is exactly the topic-level grouping Karpathy's entity pages are.
/// Every older claim a newer one demonstrably about the same episode appears to complete.
///
/// **Two groupings were tried and rejected before this one, and both failures are instructive.**
///
/// *By title* — the `subject_key` grouping contradictions use — returned **zero candidates across
/// 2,095 subjects**. It requires an identical title, and after the fold almost every subject is a
/// singleton. Correct for contradictions, useless here.
///
/// *By derived subject page* returned **2,477 candidates across 150 subjects**: noise. Those
/// subjects are single shared *terms*. `codex` alone produced 163 pairs, and `detection` grouped
/// "duplicate source_id detection" with "staging directory age detection" because both contain the
/// word. A shared word is not a shared topic.
///
/// What works is the criterion wikilinks already use: **shared evidence**. Two claims citing the
/// same event are demonstrably about the same episode — not similarly worded, *the same thing
/// happened*. So a candidate is a linked pair whose newer side rests on events the older never saw.
/// That is not a guess about language; it is a fact about what each claim was written from.
pub fn propose_revisions(
    ledger: &EventLedger,
    project_id: ProjectId,
    min_gap_days: Option<f64>,
) -> Result<RevisionReport> {
    let min_gap = min_gap_days.unwrap_or(DEFAULT_MIN_GAP_DAYS);
    let memories = ledger.current_project_memories()?;
    let by_id: std::collections::HashMap<uuid::Uuid, &MemoryRecord> =
        memories.iter().map(|memory| (memory.id, memory)).collect();

    // event -> the claims citing it. The same index the wikilink derivation builds.
    let mut citing: std::collections::HashMap<uuid::Uuid, Vec<uuid::Uuid>> =
        std::collections::HashMap::new();
    for memory in &memories {
        for event in &memory.evidence_ids {
            citing.entry(*event).or_default().push(memory.id);
        }
    }

    let mut seen_pairs: std::collections::HashSet<(uuid::Uuid, uuid::Uuid)> =
        std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for sharers in citing.values() {
        for (index, left_id) in sharers.iter().enumerate() {
            for right_id in &sharers[index + 1..] {
                let (Some(left), Some(right)) = (by_id.get(left_id), by_id.get(right_id)) else {
                    continue;
                };
                let (older, newer) = if left.valid_from <= right.valid_from {
                    (*left, *right)
                } else {
                    (*right, *left)
                };
                if older.id == newer.id || !seen_pairs.insert((older.id, newer.id)) {
                    continue;
                }
                let seen: std::collections::HashSet<uuid::Uuid> =
                    older.evidence_ids.iter().copied().collect();
                let unseen: Vec<uuid::Uuid> = newer
                    .evidence_ids
                    .iter()
                    .copied()
                    .filter(|id| !seen.contains(id))
                    .collect();
                if unseen.len() < MIN_NEW_EVIDENCE {
                    continue;
                }
                let gap_days = (newer.valid_from - older.valid_from).as_seconds_f64() / 86_400.0;
                if gap_days < min_gap {
                    continue;
                }
                candidates.push(RevisionCandidate {
                    subject: format!("{} -> {}", older.kind.as_str(), newer.kind.as_str()),
                    older_id: older.id,
                    older_title: older.title.clone(),
                    older_valid_from: older.valid_from,
                    newer_id: newer.id,
                    newer_title: newer.title.clone(),
                    newer_valid_from: newer.valid_from,
                    unseen_evidence: unseen,
                    gap_days,
                });
            }
        }
    }
    // Most unseen evidence first, then the widest time gap: the largest distance between what a
    // claim rests on and what is now known is the one most worth a person's attention.
    candidates.sort_by(|left, right| {
        right
            .unseen_evidence
            .len()
            .cmp(&left.unseen_evidence.len())
            .then_with(|| {
                right
                    .gap_days
                    .partial_cmp(&left.gap_days)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(RevisionReport {
        project_id,
        subjects_examined: citing.len(),
        candidates,
    })
}

pub fn render(report: &RevisionReport) -> String {
    if report.candidates.is_empty() {
        return format!(
            "no revision candidates — no linked pair has a newer side resting on evidence the \
             older one never saw, across {} cited events\n",
            report.subjects_examined
        );
    }
    let mut out = format!(
        "{} linked pair(s) where the newer claim rests on evidence the older never saw, across {} \
         cited events\n\n",
        report.candidates.len(),
        report.subjects_examined
    );
    for candidate in &report.candidates {
        out.push_str(&format!("{}\n", candidate.subject));
        out.push_str(&format!(
            "    older  {} — {}\n",
            candidate.older_valid_from.date(),
            candidate.older_title
        ));
        out.push_str(&format!(
            "    newer  {} — {}\n",
            candidate.newer_valid_from.date(),
            candidate.newer_title
        ));
        out.push_str(&format!(
            "  -> rests on {} event(s) the older claim never saw, {:.0} days later\n\n",
            candidate.unseen_evidence.len(),
            candidate.gap_days
        ));
    }
    out.push_str(
        "Nothing was changed. Rewriting a claim to fold in what a later one learned is a judgement \
         about language, and a model making it silently is how a verifiable system becomes a \
         plausible one. Read the unseen evidence, then file the corrected claim with \
         `brain remember --supersedes <older-id>`.\n",
    );
    out
}
