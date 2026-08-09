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
use brain_context::authority_rank;
use brain_domain::{MemoryRecord, MemoryScope, MemoryStatus, ProjectId};
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
         `brain remember --supersedes <older-id>`, or run `--propose` to have one drafted.\n",
    );
    out
}

// ---------------------------------------------------------------------------
// The rewrite half — proposing a merge, and refusing to apply an unsound one
// ---------------------------------------------------------------------------

/// What happened to one candidate when a provider was asked to merge it.
#[derive(Debug, serde::Serialize)]
pub struct MergeOutcome {
    pub older_id: uuid::Uuid,
    pub newer_id: uuid::Uuid,
    pub older_title: String,
    pub newer_title: String,
    /// The merged claim, when the provider produced one that passed every rule.
    pub merged_title: Option<String>,
    pub merged_content: Option<String>,
    /// Why it was refused, in words. Set when nothing was written.
    pub rejected: Option<String>,
    /// Whether the merge was appended, as opposed to merely proposed.
    pub applied: bool,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct MergeReport {
    pub proposed: usize,
    pub applied: usize,
    pub refused: usize,
    pub outcomes: Vec<MergeOutcome>,
}

/// Ask a provider to merge each candidate, and let derivation decide what may be written.
///
/// **`apply` is the approval, and it is per-run rather than per-pair.** A deliberate limitation
/// worth naming: reviewing thirteen proposals one at a time is a UI this does not have, so the
/// honest workflow is to read them with `--propose`, then re-run with `--apply` once they look
/// right. `--limit` keeps that a small number at a time.
///
/// A refusal records its reason on the outcome rather than raising, so one unsound proposal cannot
/// discard the sound ones beside it — the same blast-radius lesson `validate_proposed_batch`
/// learned when a single invented event id cost a job all seven of its memories.
pub async fn merge_candidates(
    ledger: &mut EventLedger,
    project_id: ProjectId,
    provider: &dyn brain_context::MergeProvider,
    candidates: &[RevisionCandidate],
    limit: usize,
    apply: bool,
    now: time::OffsetDateTime,
) -> Result<MergeReport> {
    let mut report = MergeReport::default();
    for candidate in candidates.iter().take(limit) {
        let (Some(older), Some(newer)) = (
            ledger.current_memory(candidate.older_id)?,
            ledger.current_memory(candidate.newer_id)?,
        ) else {
            continue;
        };
        let mut allowed: Vec<uuid::Uuid> = older
            .evidence_ids
            .iter()
            .chain(newer.evidence_ids.iter())
            .copied()
            .collect();
        allowed.sort_unstable();
        allowed.dedup();

        let instruction =
            brain_context::merge_instruction(&older.content, &newer.content, &allowed);
        // A provider failure is *this candidate's* failure, not the run's.
        //
        // `?` here threw away every proposal already generated: one transient
        // `operation timed out` from the endpoint discarded seven completed merges, each of
        // which had cost a call and the better part of a minute. That makes the command
        // unusable at any limit worth passing — and the failure is most likely exactly when
        // the limit is large, because the service's own consolidation loop is hitting the same
        // endpoint concurrently.
        //
        // Recorded as a rejection rather than swallowed, so the run's own report says which
        // candidates never got an answer instead of quietly returning fewer than were asked for.
        let body = match provider.merge(&instruction).await {
            Ok(body) => body,
            Err(error) => {
                report.refused += 1;
                report.outcomes.push(MergeOutcome {
                    older_id: candidate.older_id,
                    newer_id: candidate.newer_id,
                    older_title: older.title.clone(),
                    newer_title: newer.title.clone(),
                    merged_title: None,
                    merged_content: None,
                    rejected: Some(format!("provider call failed: {error}")),
                    applied: false,
                });
                continue;
            }
        };
        let checked = brain_context::parse_merge_response(&body)
            .map_err(|error| error.to_string())
            .and_then(|proposed| {
                brain_context::validate_merge(
                    &proposed,
                    &brain_context::MergeInputs {
                        older_content: &older.content,
                        newer_content: &newer.content,
                        allowed_evidence: &allowed,
                        unseen_evidence: &candidate.unseen_evidence,
                    },
                )
                .map_err(|rejection| rejection.to_string())
            });

        let outcome = match checked {
            Ok(validated) => {
                let mut applied = false;
                if apply {
                    // Supersede *both* sides. The merged claim replaces the pair; leaving either
                    // current would put three claims where there had been two.
                    let record = MemoryRecord {
                        id: uuid::Uuid::now_v7(),
                        version_id: uuid::Uuid::now_v7(),
                        scope: MemoryScope::Project(project_id),
                        worktree_id: newer.worktree_id,
                        task_id: None,
                        kind: newer.kind.clone(),
                        title: validated.title.clone(),
                        content: validated.content.clone(),
                        valid_from: newer.valid_from,
                        valid_to: None,
                        recorded_at: now,
                        confidence: newer.confidence.min(older.confidence),
                        // The higher of the two. A merge of a human correction and a derived claim
                        // is still a corrected claim, and demoting it would let the next derived
                        // memory on the subject outrank it.
                        authority: if authority_rank(&older.authority)
                            >= authority_rank(&newer.authority)
                        {
                            older.authority.clone()
                        } else {
                            newer.authority.clone()
                        },
                        evidence_ids: validated.evidence_ids.clone(),
                        supersedes: vec![older.version_id, newer.version_id],
                        status: MemoryStatus::Current,
                    };
                    ledger.append_memory(&record)?;
                    for retired in [&older, &newer] {
                        let mut version = retired.clone();
                        version.version_id = uuid::Uuid::now_v7();
                        version.recorded_at = now;
                        version.status = MemoryStatus::Superseded;
                        version.supersedes = Vec::new();
                        ledger.append_memory(&version)?;
                    }
                    applied = true;
                    report.applied += 1;
                }
                report.proposed += 1;
                MergeOutcome {
                    older_id: older.id,
                    newer_id: newer.id,
                    older_title: older.title.clone(),
                    newer_title: newer.title.clone(),
                    merged_title: Some(validated.title),
                    merged_content: Some(validated.content),
                    rejected: None,
                    applied,
                }
            }
            Err(reason) => {
                report.refused += 1;
                MergeOutcome {
                    older_id: older.id,
                    newer_id: newer.id,
                    older_title: older.title.clone(),
                    newer_title: newer.title.clone(),
                    merged_title: None,
                    merged_content: None,
                    rejected: Some(reason),
                    applied: false,
                }
            }
        };
        report.outcomes.push(outcome);
    }
    Ok(report)
}

pub fn render_merges(report: &MergeReport) -> String {
    let mut out = format!(
        "{} merge(s) proposed · {} applied · {} refused\n\n",
        report.proposed, report.applied, report.refused
    );
    for outcome in &report.outcomes {
        out.push_str(&format!(
            "  {}\n  + {}\n",
            clip(&outcome.older_title),
            clip(&outcome.newer_title)
        ));
        match (&outcome.merged_title, &outcome.rejected) {
            (Some(title), _) => out.push_str(&format!(
                "    -> {}{}\n\n",
                clip(title),
                if outcome.applied { "  [applied]" } else { "" }
            )),
            (None, Some(reason)) => out.push_str(&format!("    -> refused: {reason}\n\n")),
            _ => {}
        }
    }
    if report.applied == 0 && report.proposed > 0 {
        out.push_str("Nothing was written. Re-run with --apply once the proposals read right.\n");
    }
    out
}

fn clip(text: &str) -> String {
    if text.chars().count() <= 62 {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(61).collect::<String>())
}
