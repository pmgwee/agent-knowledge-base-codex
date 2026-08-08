//! A paragraph on a subject page, and the validation that makes one safe to publish.
//!
//! Subject pages hold only titles and links today, and that is a stronger guarantee than it looks:
//! **a page that asserts nothing cannot become wrong independently of the notes it lists.** A new
//! memory appears at the next projection; nothing revises, nothing goes stale, and no sentence can
//! drift from the evidence because there are no sentences.
//!
//! Prose gives that up. A paragraph *can* contradict the ledger, and it can do so while looking
//! exactly like the rest of the vault. So the validator here is not a formality bolted onto a
//! generation step — it is the reason the generation step is allowed to exist at all.
//!
//! Three rules, and the third is the one that matters:
//!
//! 1. Every sentence must cite at least one memory.
//! 2. Every citation must name a memory **in this subject's own set** — not merely one that exists.
//!    A model that reaches for a real memory from an unrelated subject has still written a sentence
//!    the page cannot support.
//! 3. **One bad sentence rejects the whole paragraph.** Dropping the offending sentence and keeping
//!    the rest would leave prose that reads as complete while having been quietly edited, which is
//!    worse than no prose: the reader cannot see the hole. Rejection falls back to links-only, which
//!    is the state that was already known to be safe.

use std::collections::HashSet;

use anyhow::Result;

/// A proposed paragraph, before validation.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProposedSynthesis {
    /// Sentences, each with the memories it rests on.
    pub sentences: Vec<ProposedSentence>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ProposedSentence {
    pub text: String,
    /// Memory ids this sentence rests on. Must be non-empty and drawn from the subject.
    #[serde(default)]
    pub memory_ids: Vec<uuid::Uuid>,
}

/// A paragraph that passed every rule, with the citations it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedSynthesis {
    pub sentences: Vec<ValidatedSentence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedSentence {
    pub text: String,
    pub memory_ids: Vec<uuid::Uuid>,
}

impl ValidatedSynthesis {
    /// Render as Markdown, each sentence followed by wikilinks to what it rests on.
    ///
    /// The citations are rendered inline rather than gathered into a footnote, so a reader checking
    /// one claim does not have to work out which of a page's references belongs to it.
    pub fn to_markdown(&self, titles: &dyn Fn(uuid::Uuid) -> Option<String>) -> String {
        let mut out = String::new();
        for sentence in &self.sentences {
            out.push_str(sentence.text.trim());
            let links: Vec<String> = sentence
                .memory_ids
                .iter()
                .filter_map(|id| titles(*id))
                .map(|title| format!("[[{title}]]"))
                .collect();
            if !links.is_empty() {
                out.push_str(&format!(" ({})", links.join(", ")));
            }
            out.push(' ');
        }
        out.trim_end().to_owned()
    }
}

/// Why a proposal was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SynthesisRejection {
    /// The model returned nothing to publish.
    Empty,
    /// A sentence carried no citation at all.
    UncitedSentence { text: String },
    /// A sentence cited a memory outside this subject.
    ForeignCitation { text: String, memory_id: uuid::Uuid },
    /// A sentence was longer than a sentence.
    ///
    /// Not pedantry: the unit of citation is the sentence, so a "sentence" holding four claims
    /// under one citation is three uncited claims wearing a valid one's coat.
    Overlong { text: String, characters: usize },
}

impl std::fmt::Display for SynthesisRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "no sentences proposed"),
            Self::UncitedSentence { text } => {
                write!(formatter, "sentence cites nothing: {}", clip(text))
            }
            Self::ForeignCitation { text, memory_id } => write!(
                formatter,
                "sentence cites {memory_id}, which is not in this subject: {}",
                clip(text)
            ),
            Self::Overlong { text, characters } => write!(
                formatter,
                "sentence is {characters} characters and carries more than one claim: {}",
                clip(text)
            ),
        }
    }
}

/// Longest a single cited sentence may be.
///
/// A generous bound — this is not a style rule. It exists because the citation unit is the
/// sentence: past a certain length a model has stopped writing one claim and started writing a
/// paragraph, and the single citation attached to it now covers claims it never supported.
pub const MAX_SENTENCE_CHARACTERS: usize = 400;

/// Check a proposal against the subject it claims to describe.
///
/// `subject_memories` is the *whole* rule. A citation to a memory that exists elsewhere in the
/// project is still a citation this page cannot support, and accepting it would let a subject page
/// drift into being about something else entirely — one sentence at a time, each individually
/// defensible.
pub fn validate_synthesis(
    proposed: &ProposedSynthesis,
    subject_memories: &[uuid::Uuid],
) -> std::result::Result<ValidatedSynthesis, SynthesisRejection> {
    let allowed: HashSet<uuid::Uuid> = subject_memories.iter().copied().collect();
    let sentences: Vec<&ProposedSentence> = proposed
        .sentences
        .iter()
        .filter(|sentence| !sentence.text.trim().is_empty())
        .collect();
    if sentences.is_empty() {
        return Err(SynthesisRejection::Empty);
    }

    let mut validated = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        let text = sentence.text.trim();
        if sentence.memory_ids.is_empty() {
            return Err(SynthesisRejection::UncitedSentence {
                text: text.to_owned(),
            });
        }
        if text.chars().count() > MAX_SENTENCE_CHARACTERS {
            return Err(SynthesisRejection::Overlong {
                text: text.to_owned(),
                characters: text.chars().count(),
            });
        }
        for memory_id in &sentence.memory_ids {
            if !allowed.contains(memory_id) {
                return Err(SynthesisRejection::ForeignCitation {
                    text: text.to_owned(),
                    memory_id: *memory_id,
                });
            }
        }
        let mut memory_ids = sentence.memory_ids.clone();
        memory_ids.sort_unstable();
        memory_ids.dedup();
        validated.push(ValidatedSentence {
            text: text.to_owned(),
            memory_ids,
        });
    }
    Ok(ValidatedSynthesis {
        sentences: validated,
    })
}

/// Parse a provider's JSON response into a proposal.
///
/// Tolerant of a fenced code block, because providers wrap JSON in one often enough that failing on
/// it would be reporting a formatting habit as a content failure.
pub fn parse_synthesis_response(body: &str) -> Result<ProposedSynthesis> {
    let trimmed = body.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.rsplit_once("```").map(|(head, _)| head))
        .unwrap_or(trimmed);
    serde_json::from_str(unfenced.trim()).map_err(|error| {
        anyhow::anyhow!(
            "subject synthesis response was not the expected JSON: {error}; content was: {}",
            clip(unfenced)
        )
    })
}

/// The instruction given to the provider.
///
/// Spells out the rules the validator enforces rather than leaving the model to discover them by
/// rejection. Both halves have to agree or every proposal fails and the failure looks like a model
/// problem.
pub fn synthesis_instruction(subject: &str) -> String {
    format!(
        "Write a short factual summary of what this project's memories say about \"{subject}\".\n\
         Return only strict JSON matching {{\"sentences\":[{{\"text\":..., \"memory_ids\":[...]}}]}}.\n\
         Each sentence MUST cite at least one memory_id, copied verbatim from the supplied \
         memories. Never invent one, and never cite a memory that was not supplied.\n\
         Write one claim per sentence, under {MAX_SENTENCE_CHARACTERS} characters. A sentence \
         carrying several claims under one citation is not acceptable.\n\
         State only what the supplied memories say. Do not infer, do not generalise beyond them, \
         and do not add recommendations.\n\
         Three to six sentences. Return fewer rather than padding."
    )
}

fn clip(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 120 {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(119).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(count: usize) -> Vec<uuid::Uuid> {
        (0..count).map(|_| uuid::Uuid::now_v7()).collect()
    }

    fn sentence(text: &str, memory_ids: Vec<uuid::Uuid>) -> ProposedSentence {
        ProposedSentence {
            text: text.to_owned(),
            memory_ids,
        }
    }

    #[test]
    fn a_properly_cited_paragraph_is_accepted() {
        let subject = ids(3);
        let proposed = ProposedSynthesis {
            sentences: vec![
                sentence("Deployment happens on commit.", vec![subject[0]]),
                sentence("The build must pass first.", vec![subject[1], subject[2]]),
            ],
        };
        let validated = validate_synthesis(&proposed, &subject).expect("accepted");
        assert_eq!(validated.sentences.len(), 2);
        assert_eq!(validated.sentences[1].memory_ids.len(), 2);
    }

    #[test]
    fn an_uncited_sentence_rejects_the_whole_paragraph() {
        // Not just the sentence. Dropping it silently would leave prose that reads as complete
        // while having been edited, and the reader cannot see the hole.
        let subject = ids(2);
        let proposed = ProposedSynthesis {
            sentences: vec![
                sentence("Deployment happens on commit.", vec![subject[0]]),
                sentence("This is probably a good idea.", Vec::new()),
            ],
        };
        assert!(matches!(
            validate_synthesis(&proposed, &subject),
            Err(SynthesisRejection::UncitedSentence { .. })
        ));
    }

    #[test]
    fn a_citation_to_a_real_memory_outside_the_subject_is_still_refused() {
        // The subtle one. The id resolves, the memory exists, the claim may even be true — and the
        // page still cannot support it. Accepting these lets a subject page drift into being about
        // something else, one individually defensible sentence at a time.
        let subject = ids(2);
        let elsewhere = uuid::Uuid::now_v7();
        let proposed = ProposedSynthesis {
            sentences: vec![sentence("Deployment happens on commit.", vec![elsewhere])],
        };
        match validate_synthesis(&proposed, &subject) {
            Err(SynthesisRejection::ForeignCitation { memory_id, .. }) => {
                assert_eq!(memory_id, elsewhere);
            }
            other => panic!("expected a foreign-citation rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_sentence_carrying_a_paragraph_of_claims_is_refused() {
        let subject = ids(1);
        let long = "x".repeat(MAX_SENTENCE_CHARACTERS + 1);
        let proposed = ProposedSynthesis {
            sentences: vec![sentence(&long, vec![subject[0]])],
        };
        assert!(matches!(
            validate_synthesis(&proposed, &subject),
            Err(SynthesisRejection::Overlong { .. })
        ));
    }

    #[test]
    fn an_empty_or_blank_proposal_is_refused_rather_than_published() {
        let subject = ids(1);
        assert_eq!(
            validate_synthesis(
                &ProposedSynthesis {
                    sentences: Vec::new()
                },
                &subject
            ),
            Err(SynthesisRejection::Empty)
        );
        assert_eq!(
            validate_synthesis(
                &ProposedSynthesis {
                    sentences: vec![sentence("   ", vec![subject[0]])],
                },
                &subject
            ),
            Err(SynthesisRejection::Empty)
        );
    }

    #[test]
    fn duplicate_citations_collapse_rather_than_repeating_a_link() {
        let subject = ids(1);
        let proposed = ProposedSynthesis {
            sentences: vec![sentence(
                "Deployment happens on commit.",
                vec![subject[0], subject[0]],
            )],
        };
        let validated = validate_synthesis(&proposed, &subject).expect("accepted");
        assert_eq!(validated.sentences[0].memory_ids, vec![subject[0]]);
    }

    #[test]
    fn markdown_renders_each_sentence_with_its_own_citations() {
        let subject = ids(2);
        let validated = validate_synthesis(
            &ProposedSynthesis {
                sentences: vec![
                    sentence("Deployment happens on commit.", vec![subject[0]]),
                    sentence("The build must pass first.", vec![subject[1]]),
                ],
            },
            &subject,
        )
        .expect("accepted");

        let first = subject[0];
        let rendered = validated.to_markdown(&|id| {
            Some(if id == first {
                "Deploy on commit".to_owned()
            } else {
                "Build gate".to_owned()
            })
        });
        assert_eq!(
            rendered,
            "Deployment happens on commit. ([[Deploy on commit]]) The build must pass first. \
             ([[Build gate]])"
        );
    }

    #[test]
    fn a_fenced_response_parses() {
        let id = uuid::Uuid::now_v7();
        let body = format!(
            "```json\n{{\"sentences\":[{{\"text\":\"A claim.\",\"memory_ids\":[\"{id}\"]}}]}}\n```"
        );
        let parsed = parse_synthesis_response(&body).expect("parse");
        assert_eq!(parsed.sentences.len(), 1);
        assert_eq!(parsed.sentences[0].memory_ids, vec![id]);
    }

    #[test]
    fn a_response_missing_citations_parses_and_is_then_refused() {
        // The two halves are separate on purpose: a proposal that *parses* has not been *accepted*,
        // and conflating them is how an uncited claim would reach a page.
        let body = "{\"sentences\":[{\"text\":\"A claim with no citation.\"}]}";
        let parsed = parse_synthesis_response(body).expect("parse");
        assert!(matches!(
            validate_synthesis(&parsed, &ids(1)),
            Err(SynthesisRejection::UncitedSentence { .. })
        ));
    }

    #[test]
    fn a_non_json_response_names_what_it_saw() {
        let error = parse_synthesis_response("I'm sorry, I cannot do that").expect_err("reject");
        assert!(format!("{error}").contains("I'm sorry"));
    }

    #[test]
    fn the_instruction_states_the_limit_the_validator_enforces() {
        // If these drift apart every proposal fails and the failure reads as a model problem.
        let instruction = synthesis_instruction("deployment");
        assert!(instruction.contains(&MAX_SENTENCE_CHARACTERS.to_string()));
        assert!(instruction.contains("deployment"));
    }
}
