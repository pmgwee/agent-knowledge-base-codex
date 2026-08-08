//! Health-checking the vault.
//!
//! Karpathy's third operation, and the one this brain had none of. Ingest and query both run; there
//! was nothing that periodically asked *"is what we have built still coherent"* — contradictions
//! between pages, claims newer sources have superseded, subjects mentioned everywhere with no page
//! of their own, notes nothing links to.
//!
//! Every check here is **derived**, and that is the constraint that shapes all of them. A lint that
//! asked a model "does this look wrong to you" would produce findings with no evidence behind them,
//! in a vault whose entire property is that nothing in it is unsourced. So each finding names the
//! rule it failed and the memories it concerns, and a human decides.
//!
//! It is also deliberately quiet about things that are *fine*. An orphan memory is not a bug here —
//! it genuinely shares no evidence with any other, which happens and is honest. It is reported as a
//! count and a sample, not as an error, because the useful signal is the trend rather than the
//! individual note.

use anyhow::Result;
use brain_context::resolve_candidates;
use brain_domain::ProjectId;
use brain_store::EventLedger;

/// How old a memory's newest cited event must be before the claim counts as unrefreshed.
///
/// Not "wrong" — unrefreshed. A decision made in March and never revisited since may be perfectly
/// current; the point is that nothing has confirmed it, and on a fast-moving project that is worth
/// knowing.
const STALE_AFTER_DAYS: i64 = 60;

/// Findings shown per category before the report summarises the rest.
///
/// A lint that prints two hundred lines is one nobody reads twice. The total is always stated so a
/// truncated list never implies the vault is healthier than it is.
const MAX_EXAMPLES: usize = 8;

#[derive(Debug, serde::Serialize)]
pub struct LintReport {
    pub project_id: ProjectId,
    pub memories_checked: usize,
    pub findings: Vec<LintFinding>,
    /// Whether anything needs a human. Counts and observations do not; contradictions do.
    pub actionable: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct LintFinding {
    pub rule: &'static str,
    /// What the rule means, in one sentence, so a report is readable without the source.
    pub explanation: String,
    pub count: usize,
    /// Up to [`MAX_EXAMPLES`] concrete instances.
    pub examples: Vec<String>,
    /// Whether this needs a decision, as opposed to being worth knowing.
    pub actionable: bool,
}

pub fn lint_project(
    ledger: &EventLedger,
    project_id: ProjectId,
    now: time::OffsetDateTime,
) -> Result<LintReport> {
    let memories = ledger.current_project_memories()?;
    let mut findings = Vec::new();

    // 1. Contradictions. The resolver already computes these — two memories about the same
    //    subject, at equal authority, saying different things. Nothing surfaced them outside an
    //    orientation, where they compete for the same scarce space as everything else.
    let resolved = resolve_candidates(project_id, now, memories.clone());
    if !resolved.conflicts.is_empty() {
        findings.push(LintFinding {
            rule: "contradiction",
            explanation: "Two or more current memories about the same subject disagree, and \
                          neither outranks the other. Supersede one, or correct both."
                .to_owned(),
            count: resolved.conflicts.len(),
            examples: resolved
                .conflicts
                .iter()
                .take(MAX_EXAMPLES)
                .map(|conflict| {
                    format!(
                        "{} — {} memories disagree",
                        conflict.subject,
                        conflict.records.len()
                    )
                })
                .collect(),
            actionable: true,
        });
    }

    // 2. Misdated claims. Found by running this against the live brain: four memories carried
    //    `valid_from` of 1970-01-01, which is the Unix epoch and therefore a default rather than
    //    a date. They are not old, they are undated — and they sort to the front of every
    //    chronological view, which is the opposite of what an unset field should do.
    let epoch_ish = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(365);
    let misdated: Vec<&brain_domain::MemoryRecord> = memories
        .iter()
        .filter(|memory| memory.valid_from < epoch_ish)
        .collect();
    if !misdated.is_empty() {
        findings.push(LintFinding {
            rule: "misdated",
            explanation: "`valid_from` is at or near the Unix epoch, which is a default rather \
                          than a date. The provider did not supply one, and these sort to the \
                          front of every chronological view as a result."
                .to_owned(),
            count: misdated.len(),
            examples: misdated
                .iter()
                .take(MAX_EXAMPLES)
                .map(|memory| format!("{} — {}", memory.valid_from.date(), memory.title))
                .collect(),
            actionable: true,
        });
    }

    // 3. Unrefreshed claims. Age alone is not staleness — a decision can be years old and still
    //    correct — so this reports it as something to know rather than something to fix.
    let cutoff = now - time::Duration::days(STALE_AFTER_DAYS);
    let mut unrefreshed: Vec<&brain_domain::MemoryRecord> = memories
        .iter()
        .filter(|memory| memory.valid_from < cutoff && memory.valid_from >= epoch_ish)
        .collect();
    if !unrefreshed.is_empty() {
        unrefreshed.sort_by_key(|memory| memory.valid_from);
        findings.push(LintFinding {
            rule: "unrefreshed",
            explanation: format!(
                "Nothing has confirmed or contradicted these claims in {STALE_AFTER_DAYS} days. \
                 Old is not wrong; this is a reading list, not a defect list."
            ),
            count: unrefreshed.len(),
            examples: unrefreshed
                .iter()
                .take(MAX_EXAMPLES)
                .map(|memory| format!("{} — {}", memory.valid_from.date(), memory.title))
                .collect(),
            actionable: false,
        });
    }

    // 4. Memories with no vector. They cannot be found by meaning, cannot join a subject page,
    //    and nothing else says so — on a brain mid-backfill this is the honest explanation for a
    //    thinner vault than expected.
    let (embedded, remaining) = ledger.embedding_coverage()?;
    if remaining > 0 {
        findings.push(LintFinding {
            rule: "unembedded",
            explanation: "These memories have no vector, so they are unreachable by meaning and \
                          cannot appear on a subject page. Usually a backfill still running."
                .to_owned(),
            count: usize::try_from(remaining).unwrap_or(0),
            examples: vec![format!(
                "{embedded} embedded, {remaining} awaiting a vector"
            )],
            actionable: false,
        });
    }

    // 5. Orphans — memories sharing evidence with nothing else, so no wikilink reaches them.
    //    Reported as a proportion because the individual note is rarely the problem: a rising
    //    share means consolidation is producing isolated claims rather than connected ones.
    let orphans = count_orphans(ledger)?;
    if orphans > 0 && !memories.is_empty() {
        let share = (orphans as f64 / memories.len() as f64) * 100.0;
        findings.push(LintFinding {
            rule: "unlinked",
            explanation: "These memories share evidence with no other, so no wikilink reaches \
                          them and Obsidian's graph shows them as islands. Honest when a claim \
                          really is isolated; a rising share means consolidation is fragmenting."
                .to_owned(),
            count: orphans,
            examples: vec![format!(
                "{orphans} of {} memories ({share:.1}%)",
                memories.len()
            )],
            actionable: false,
        });
    }

    // 6. Withdrawn memories, so a vault that looks smaller than the ledger has an explanation
    //    rather than a discrepancy.
    let tombstones = ledger.tombstones()?;
    if !tombstones.is_empty() {
        findings.push(LintFinding {
            rule: "withdrawn",
            explanation: "Deliberately withdrawn and excluded from every read path. Listed so a \
                          vault smaller than the ledger has an explanation."
                .to_owned(),
            count: tombstones.len(),
            examples: tombstones
                .iter()
                .take(MAX_EXAMPLES)
                .map(|tombstone| {
                    format!(
                        "{} — {} (by {})",
                        tombstone.redacted_at.date(),
                        tombstone.reason,
                        tombstone.redacted_by
                    )
                })
                .collect(),
            actionable: false,
        });
    }

    let actionable = findings.iter().any(|finding| finding.actionable);
    Ok(LintReport {
        project_id,
        memories_checked: memories.len(),
        findings,
        actionable,
    })
}

/// Memories citing no event that any other memory also cites.
fn count_orphans(ledger: &EventLedger) -> Result<usize> {
    ledger.memories_without_shared_evidence()
}

/// Render the report for a terminal.
pub fn render(report: &LintReport) -> String {
    let mut out = format!(
        "lint {} — {} current memories\n",
        report.project_id.0, report.memories_checked
    );
    if report.findings.is_empty() {
        out.push_str("\nNothing to report.\n");
        return out;
    }
    for finding in &report.findings {
        out.push_str(&format!(
            "\n{} {} ({})\n  {}\n",
            if finding.actionable { "!" } else { "·" },
            finding.rule,
            finding.count,
            finding.explanation,
        ));
        for example in &finding.examples {
            out.push_str(&format!("    {example}\n"));
        }
        if finding.count > finding.examples.len() && finding.examples.len() >= MAX_EXAMPLES {
            out.push_str(&format!(
                "    …and {} more\n",
                finding.count - finding.examples.len()
            ));
        }
    }
    if !report.actionable {
        out.push_str("\nNothing here needs a decision.\n");
    }
    out
}
