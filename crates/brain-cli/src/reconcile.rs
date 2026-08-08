//! Proposing a resolution to a contradiction, and refusing to invent one.
//!
//! `brain lint` finds memories that disagree and stops. Stopping is defensible — resolving means
//! deciding which claim is true, and a model deciding that leaves no evidence trail, which converts
//! a verifiable system into a plausible one. But "detect and stop" is not the only alternative to
//! "let a model choose".
//!
//! This proposes. Every proposal is **derived from facts already in the ledger**, in a fixed order,
//! and the rule that decided it is printed beside it. Nothing is applied without an explicit
//! `--apply`, and anything the rules cannot separate is reported as needing a human rather than
//! guessed at.
//!
//! The ordering is the whole design:
//!
//! 1. **Authority.** A human correction outranks an agent checkpoint outranks a derived memory. This
//!    is the existing `authority_rank`, not a new opinion.
//! 2. **Recency**, when authority ties. The later `valid_from` wins — a claim made with knowledge of
//!    the earlier one.
//! 3. **Evidence weight**, when both tie. More cited events is a weaker signal than the other two
//!    and is deliberately last.
//! 4. **Otherwise, no proposal.** Two derived memories, same day, same evidence count, flatly
//!    disagreeing is exactly the case a human should see.

use anyhow::Result;
use brain_context::{authority_rank, resolve_candidates};
use brain_domain::{MemoryRecord, ProjectId};
use brain_store::EventLedger;

/// Why one memory was proposed over the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionRule {
    /// A higher authority stated it.
    Authority,
    /// Same authority; this one is newer.
    Recency,
    /// Same authority and date; this one rests on more evidence.
    EvidenceWeight,
}

impl ResolutionRule {
    pub fn explain(self) -> &'static str {
        match self {
            Self::Authority => "stated by a higher authority",
            Self::Recency => "same authority, and this claim is newer",
            Self::EvidenceWeight => "same authority and date, and this rests on more evidence",
        }
    }
}

/// A contradiction, and what derivation would do about it.
#[derive(Debug, serde::Serialize)]
pub struct Proposal {
    pub subject: String,
    /// The memory that would remain current, when the rules can pick one.
    pub keep: Option<uuid::Uuid>,
    pub keep_title: Option<String>,
    /// Memories the keeper would supersede.
    pub supersede: Vec<uuid::Uuid>,
    pub rule: Option<ResolutionRule>,
    /// Every side of the disagreement, so the report stands alone.
    pub sides: Vec<String>,
}

impl Proposal {
    /// Whether the rules separated the sides at all.
    pub fn is_decided(&self) -> bool {
        self.keep.is_some()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ReconcileReport {
    pub project_id: ProjectId,
    pub contradictions: usize,
    pub proposals: Vec<Proposal>,
    /// Contradictions the rules could not separate. These need a human, and saying so is the point.
    pub undecidable: usize,
}

/// Examine every contradiction and propose what derivation would do. Changes nothing.
pub fn propose(ledger: &EventLedger, project_id: ProjectId) -> Result<ReconcileReport> {
    let memories = ledger.current_project_memories()?;
    let resolved = resolve_candidates(project_id, time::OffsetDateTime::now_utc(), memories);

    let mut proposals = Vec::new();
    let mut undecidable = 0;
    for conflict in &resolved.conflicts {
        let proposal = propose_for(&conflict.subject, &conflict.records);
        if !proposal.is_decided() {
            undecidable += 1;
        }
        proposals.push(proposal);
    }
    Ok(ReconcileReport {
        project_id,
        contradictions: resolved.conflicts.len(),
        proposals,
        undecidable,
    })
}

/// The rules, applied in order, to one contradiction.
///
/// Returns an undecided proposal rather than picking arbitrarily when the sides are level. A
/// coin-flip dressed as a rule is worse than an honest "you decide": it would be applied silently
/// and never revisited.
fn propose_for(subject: &str, records: &[MemoryRecord]) -> Proposal {
    let sides: Vec<String> = records
        .iter()
        .map(|record| {
            format!(
                "{} [{}, {}, {} evidence] — {}",
                record.title,
                record.authority.as_str(),
                record.valid_from.date(),
                record.evidence_ids.len(),
                record.id
            )
        })
        .collect();

    let undecided = |sides: Vec<String>| Proposal {
        subject: subject.to_owned(),
        keep: None,
        keep_title: None,
        supersede: Vec::new(),
        rule: None,
        sides,
    };
    if records.len() < 2 {
        return undecided(sides);
    }

    // Each rule produces a score; the first rule that yields a *unique* maximum decides.
    let by_authority = |record: &MemoryRecord| authority_rank(&record.authority) as i64;
    let by_recency = |record: &MemoryRecord| record.valid_from.unix_timestamp();
    let by_evidence = |record: &MemoryRecord| record.evidence_ids.len() as i64;

    for (rule, score) in [
        (
            ResolutionRule::Authority,
            &by_authority as &dyn Fn(&MemoryRecord) -> i64,
        ),
        (ResolutionRule::Recency, &by_recency),
        (ResolutionRule::EvidenceWeight, &by_evidence),
    ] {
        let best = records.iter().map(score).max().unwrap_or(0);
        let winners: Vec<&MemoryRecord> = records
            .iter()
            .filter(|record| score(record) == best)
            .collect();
        if winners.len() == 1 {
            let keep = winners[0];
            return Proposal {
                subject: subject.to_owned(),
                keep: Some(keep.id),
                keep_title: Some(keep.title.clone()),
                supersede: records
                    .iter()
                    .filter(|record| record.id != keep.id)
                    .map(|record| record.id)
                    .collect(),
                rule: Some(rule),
                sides,
            };
        }
    }
    undecided(sides)
}

/// Render a report for a human to act on.
pub fn render(report: &ReconcileReport) -> String {
    if report.contradictions == 0 {
        return "no contradictions — nothing to reconcile\n".to_owned();
    }
    let mut out = format!(
        "{} contradiction{} — {} with a derived proposal, {} needing a decision\n\n",
        report.contradictions,
        if report.contradictions == 1 { "" } else { "s" },
        report.contradictions - report.undecidable,
        report.undecidable
    );
    for proposal in &report.proposals {
        out.push_str(&format!("{}\n", proposal.subject));
        for side in &proposal.sides {
            out.push_str(&format!("    {side}\n"));
        }
        match (&proposal.keep_title, proposal.rule) {
            (Some(title), Some(rule)) => out.push_str(&format!(
                "  -> keep \"{title}\" — {}; supersede {} other{}\n\n",
                rule.explain(),
                proposal.supersede.len(),
                if proposal.supersede.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )),
            _ => out.push_str(
                "  -> no proposal: the sides are level on authority, date and evidence. \
                 This one is yours.\n\n",
            ),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use brain_domain::{Authority, MemoryKind, MemoryScope, MemoryStatus};

    fn record(title: &str, authority: Authority, day: i64, evidence: usize) -> MemoryRecord {
        MemoryRecord {
            id: uuid::Uuid::now_v7(),
            version_id: uuid::Uuid::now_v7(),
            scope: MemoryScope::Project(ProjectId(uuid::Uuid::nil())),
            worktree_id: None,
            task_id: None,
            kind: MemoryKind::Decision,
            title: title.to_owned(),
            content: title.to_owned(),
            valid_from: time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(day),
            valid_to: None,
            recorded_at: time::OffsetDateTime::UNIX_EPOCH,
            confidence: 1.0,
            authority,
            evidence_ids: (0..evidence).map(|_| uuid::Uuid::now_v7()).collect(),
            supersedes: Vec::new(),
            status: MemoryStatus::Current,
        }
    }

    #[test]
    fn a_human_correction_outranks_a_derived_memory_whatever_the_dates() {
        // Authority first, and deliberately ahead of recency: a later derived claim does not
        // overturn something a human stated.
        let human = record("Human said", Authority::HumanCorrection, 1, 1);
        let derived = record("Derived later", Authority::DerivedMemory, 99, 9);
        let keep_id = human.id;
        let proposal = propose_for("subject", &[derived, human]);
        assert_eq!(proposal.keep, Some(keep_id));
        assert_eq!(proposal.rule, Some(ResolutionRule::Authority));
    }

    #[test]
    fn equal_authority_falls_through_to_recency() {
        let older = record("Older", Authority::DerivedMemory, 1, 5);
        let newer = record("Newer", Authority::DerivedMemory, 50, 1);
        let keep_id = newer.id;
        let proposal = propose_for("subject", &[older, newer]);
        assert_eq!(proposal.keep, Some(keep_id));
        assert_eq!(proposal.rule, Some(ResolutionRule::Recency));
    }

    #[test]
    fn evidence_weight_is_the_last_resort_and_not_the_first() {
        let thin = record("Thin", Authority::DerivedMemory, 7, 1);
        let thick = record("Thick", Authority::DerivedMemory, 7, 20);
        let keep_id = thick.id;
        let proposal = propose_for("subject", &[thin, thick]);
        assert_eq!(proposal.keep, Some(keep_id));
        assert_eq!(proposal.rule, Some(ResolutionRule::EvidenceWeight));
    }

    #[test]
    fn level_sides_get_no_proposal_rather_than_a_coin_flip() {
        // The case the whole design exists for. A coin-flip dressed as a rule would be applied
        // silently and never revisited.
        let left = record("Left", Authority::DerivedMemory, 7, 3);
        let right = record("Right", Authority::DerivedMemory, 7, 3);
        let proposal = propose_for("subject", &[left, right]);
        assert!(!proposal.is_decided());
        assert!(proposal.rule.is_none());
        assert_eq!(proposal.sides.len(), 2, "both sides still reported");
    }

    #[test]
    fn every_side_is_reported_even_when_one_is_chosen() {
        // A proposal a reader cannot check is a decision, not a proposal.
        let human = record("Human", Authority::HumanCorrection, 1, 1);
        let derived = record("Derived", Authority::DerivedMemory, 9, 1);
        let proposal = propose_for("subject", &[human, derived]);
        assert_eq!(proposal.sides.len(), 2);
        assert!(
            proposal
                .sides
                .iter()
                .any(|s| s.contains("human_correction"))
        );
        assert!(proposal.sides.iter().any(|s| s.contains("derived_memory")));
    }
}
