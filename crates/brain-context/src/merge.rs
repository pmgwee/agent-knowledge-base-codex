//! Merging two claims into the one that is true now — and refusing everything else.
//!
//! `brain revise` finds pairs where a later claim rests on evidence an earlier one never saw. This
//! is what turns such a pair into a single corrected claim, and it is the last operation of
//! Karpathy's pattern: *"it integrates it into the existing wiki."*
//!
//! **The model gets one narrow job and four derived refusals.** It receives two claims and the union
//! of their evidence, and nothing else — so it cannot reach for context it was not given. What comes
//! back is checked against rows the ledger already holds, before anything is written:
//!
//! | Rule | Why |
//! |---|---|
//! | Citations ⊆ union of both claims' evidence | A merge cannot invent provenance |
//! | Shorter than the two inputs combined | A merge that grows is a concatenation wearing a merge's name |
//! | Retains the newer claim's unseen evidence | Otherwise it is the older claim restated and nothing was learnt |
//! | Non-empty title and content | The floor `remember` already enforces |
//!
//! A rejection **writes nothing and says why**. That is not politeness: three consolidation jobs
//! dead-lettered in this system storing 400 characters of plausible JSON and no reason at all,
//! because `fail()` kept only anyhow's top-level message. An instrument that reports confidently and
//! uselessly is worse than one that reports nothing.

use std::collections::HashSet;

use async_trait::async_trait;

/// What a provider proposed for a pair.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProposedMerge {
    pub title: String,
    pub content: String,
    /// The events this merged claim rests on. Must be a subset of the pair's combined evidence.
    #[serde(default)]
    pub evidence_ids: Vec<uuid::Uuid>,
}

/// A proposal that passed every rule, and is therefore safe to append.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ValidatedMerge {
    pub title: String,
    pub content: String,
    pub evidence_ids: Vec<uuid::Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum MergeRejection {
    EmptyTitle,
    EmptyContent,
    /// Cited an event neither claim rests on.
    ForeignCitation {
        event_id: uuid::Uuid,
    },
    /// Cited nothing at all.
    Uncited,
    /// Longer than the two claims it replaces.
    NotShorter {
        merged: usize,
        inputs: usize,
    },
    /// Dropped the very evidence that made this a revision candidate.
    LostNewEvidence {
        missing: Vec<uuid::Uuid>,
    },
}

impl std::fmt::Display for MergeRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => write!(formatter, "the merged claim has no title"),
            Self::EmptyContent => write!(formatter, "the merged claim has no content"),
            Self::Uncited => write!(
                formatter,
                "the merged claim cites nothing — an uncited claim is what this brain does not store"
            ),
            Self::ForeignCitation { event_id } => write!(
                formatter,
                "cites event:{event_id}, which neither claim rests on — a merge cannot invent \
                 provenance"
            ),
            Self::NotShorter { merged, inputs } => write!(
                formatter,
                "merged claim is {merged} characters against {inputs} for the two it replaces; a \
                 merge that grows is a concatenation"
            ),
            Self::LostNewEvidence { missing } => write!(
                formatter,
                "dropped {} event(s) that only the newer claim saw — those are the reason this pair \
                 was a candidate at all",
                missing.len()
            ),
        }
    }
}

/// What the pair being merged looked like, so validation can be a pure function of it.
pub struct MergeInputs<'a> {
    pub older_content: &'a str,
    pub newer_content: &'a str,
    /// Every event either claim cites.
    pub allowed_evidence: &'a [uuid::Uuid],
    /// Events only the newer claim saw. The merged claim must keep citing these.
    pub unseen_evidence: &'a [uuid::Uuid],
}

/// Check a proposed merge against the pair it claims to replace.
pub fn validate_merge(
    proposed: &ProposedMerge,
    inputs: &MergeInputs<'_>,
) -> std::result::Result<ValidatedMerge, MergeRejection> {
    let title = proposed.title.trim();
    let content = proposed.content.trim();
    if title.is_empty() {
        return Err(MergeRejection::EmptyTitle);
    }
    if content.is_empty() {
        return Err(MergeRejection::EmptyContent);
    }
    if proposed.evidence_ids.is_empty() {
        return Err(MergeRejection::Uncited);
    }

    let allowed: HashSet<uuid::Uuid> = inputs.allowed_evidence.iter().copied().collect();
    for event_id in &proposed.evidence_ids {
        if !allowed.contains(event_id) {
            return Err(MergeRejection::ForeignCitation {
                event_id: *event_id,
            });
        }
    }

    // Measured in characters rather than words: a merge is allowed to be *as long as* either input,
    // and the bound only catches the case where a model concatenated both and called it a synthesis.
    let inputs_length =
        inputs.older_content.trim().chars().count() + inputs.newer_content.trim().chars().count();
    let merged_length = content.chars().count();
    if merged_length >= inputs_length {
        return Err(MergeRejection::NotShorter {
            merged: merged_length,
            inputs: inputs_length,
        });
    }

    let cited: HashSet<uuid::Uuid> = proposed.evidence_ids.iter().copied().collect();
    let missing: Vec<uuid::Uuid> = inputs
        .unseen_evidence
        .iter()
        .copied()
        .filter(|id| !cited.contains(id))
        .collect();
    if !missing.is_empty() {
        return Err(MergeRejection::LostNewEvidence { missing });
    }

    let mut evidence_ids = proposed.evidence_ids.clone();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    Ok(ValidatedMerge {
        title: title.to_owned(),
        content: content.to_owned(),
        evidence_ids,
    })
}

/// Parse a provider's JSON response into a proposal.
///
/// Tolerant of a fenced code block, for the same reason `parse_synthesis_response` is: providers
/// wrap JSON in one often enough that failing on it would report a formatting habit as a content
/// failure.
pub fn parse_merge_response(body: &str) -> anyhow::Result<ProposedMerge> {
    let trimmed = body.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.rsplit_once("```").map(|(body, _)| body))
        .unwrap_or(trimmed);
    Ok(serde_json::from_str(unfenced.trim())?)
}

/// The instruction sent with a pair.
///
/// It names the refusals explicitly. A model told the rules up front fails them less often than one
/// corrected afterwards, and every rejection here costs a round trip against a quota-limited
/// provider.
pub fn merge_instruction(older: &str, newer: &str, allowed: &[uuid::Uuid]) -> String {
    let citations = allowed
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("\n  ");
    format!(
        "Two claims from one knowledge base describe the same episode. The second was written later \
         and rests on evidence the first never saw.\n\n\
         Write the ONE claim that is true now. Merge them; do not summarise either separately, and \
         do not add anything neither states.\n\n\
         EARLIER CLAIM:\n{older}\n\n\
         LATER CLAIM:\n{newer}\n\n\
         You may cite only these events:\n  {citations}\n\n\
         Reply with JSON only: {{\"title\": \"...\", \"content\": \"...\", \"evidence_ids\": \
         [\"...\"]}}\n\n\
         It will be rejected unless: every evidence_id is from the list above; the content is \
         SHORTER than the two claims combined; and it still cites the events only the later claim \
         saw."
    )
}

/// A provider that can merge two claims.
///
/// A trait rather than a concrete client for one reason: **the apply path must be testable without
/// a provider**. The live generation is deliberately held until quota — watching the citation check
/// refuse a *real* bad citation from a *real* provider is the point of having the check — but
/// everything between the proposal and the ledger can be, and is, exercised now against a stub.
/// That is the same posture 3.2b took, and for the same reason.
#[async_trait]
pub trait MergeProvider: Send + Sync {
    /// Returns the provider's raw response body, for `parse_merge_response` to interpret.
    async fn merge(&self, instruction: &str) -> anyhow::Result<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<uuid::Uuid> {
        (0..n)
            .map(|i| {
                let mut bytes = [0_u8; 16];
                bytes[0] = i as u8 + 1;
                uuid::Uuid::from_bytes(bytes)
            })
            .collect()
    }

    fn inputs<'a>(allowed: &'a [uuid::Uuid], unseen: &'a [uuid::Uuid]) -> MergeInputs<'a> {
        MergeInputs {
            older_content: "the deploy hook runs on commit",
            newer_content: "the deploy hook runs on commit and restarts the service afterwards",
            allowed_evidence: allowed,
            unseen_evidence: unseen,
        }
    }

    #[test]
    fn a_well_formed_merge_passes() {
        let all = ids(4);
        let proposed = ProposedMerge {
            title: "Committing deploys and restarts".to_owned(),
            content: "The post-commit hook builds, installs, and restarts.".to_owned(),
            evidence_ids: all.clone(),
        };
        let validated = validate_merge(&proposed, &inputs(&all, &all[2..])).expect("valid");
        assert_eq!(validated.evidence_ids.len(), 4);
    }

    #[test]
    fn a_citation_neither_claim_rests_on_is_refused() {
        // The rule that keeps a merge from inventing provenance. Without it, the one operation that
        // rewrites a claim would also be the one able to attach it to anything.
        let all = ids(4);
        let stranger = ids(9)[8];
        let proposed = ProposedMerge {
            title: "Merged".to_owned(),
            content: "Short.".to_owned(),
            evidence_ids: vec![all[0], stranger],
        };
        assert_eq!(
            validate_merge(&proposed, &inputs(&all, &all[2..])),
            Err(MergeRejection::ForeignCitation { event_id: stranger })
        );
    }

    #[test]
    fn a_merge_that_grew_is_a_concatenation() {
        let all = ids(4);
        let proposed = ProposedMerge {
            title: "Merged".to_owned(),
            content: "the deploy hook runs on commit. the deploy hook runs on commit and restarts \
                      the service afterwards. both of these remain true."
                .to_owned(),
            evidence_ids: all.clone(),
        };
        assert!(matches!(
            validate_merge(&proposed, &inputs(&all, &all[2..])),
            Err(MergeRejection::NotShorter { .. })
        ));
    }

    #[test]
    fn dropping_the_new_evidence_defeats_the_point() {
        // A merge citing only what the older claim already cited has not integrated anything — it
        // has restated the older claim and thrown away the reason the pair was a candidate.
        let all = ids(4);
        let proposed = ProposedMerge {
            title: "Merged".to_owned(),
            content: "Short.".to_owned(),
            evidence_ids: all[..2].to_vec(),
        };
        assert_eq!(
            validate_merge(&proposed, &inputs(&all, &all[2..])),
            Err(MergeRejection::LostNewEvidence {
                missing: all[2..].to_vec()
            })
        );
    }

    #[test]
    fn an_uncited_merge_is_refused() {
        let all = ids(4);
        let proposed = ProposedMerge {
            title: "Merged".to_owned(),
            content: "Short.".to_owned(),
            evidence_ids: Vec::new(),
        };
        assert_eq!(
            validate_merge(&proposed, &inputs(&all, &all[2..])),
            Err(MergeRejection::Uncited)
        );
    }

    #[test]
    fn a_fenced_response_parses() {
        let body = "```json\n{\"title\":\"T\",\"content\":\"C\",\"evidence_ids\":[]}\n```";
        assert_eq!(parse_merge_response(body).expect("parse").title, "T");
    }

    #[test]
    fn the_instruction_names_every_refusal() {
        // A model told the rules up front fails them less often than one corrected afterwards, and
        // every rejection costs a round trip against a quota-limited provider.
        let instruction = merge_instruction("older", "newer", &ids(2));
        assert!(instruction.contains("SHORTER"));
        assert!(instruction.contains("only these events"));
        assert!(instruction.contains("only the later claim"));
    }
}
