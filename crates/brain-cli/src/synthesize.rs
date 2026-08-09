//! Subject-page prose — the generation half of 3.2b.
//!
//! The validator, the store and the projector's read path all shipped months ago; nothing ever
//! called the generator. That was deliberate: watching the citation check refuse a *real* bad
//! citation from a *real* provider is the point of having one, and a stub would have proved the
//! opposite of what the check exists for. This is that call, now that the provider answers.
//!
//! A subject page without prose is a list of memory titles. With prose it is a page that says what
//! the project currently believes about that subject — which is the thing a wiki has and a folder
//! of notes does not.
//!
//! **Nothing is written that fails validation.** Every sentence must cite at least one of *this
//! subject's* memories: a citation to a memory that exists elsewhere in the project is still a
//! citation this page cannot support, and accepting it would let a subject page drift into being
//! about something else one defensible sentence at a time.

use anyhow::Result;
use brain_context::MergeProvider;
use brain_store::{EventLedger, MAX_SUBJECTS, SubjectInput, derive_subjects};

/// How much of each memory the provider is shown.
///
/// Titles alone produce prose that restates titles. Full bodies for a forty-memory subject blow
/// past any sensible request size, and the tail of a long memory is rarely what the subject is
/// about. This is the middle that keeps the request bounded without reducing it to a rename.
const MEMORY_EXCERPT_CHARACTERS: usize = 600;

/// Memories shown per subject. A subject with more than this is summarised from its newest, which
/// is the same order the page itself renders in.
const MAX_MEMORIES_PER_SUBJECT: usize = 24;

#[derive(Clone, Debug, serde::Serialize)]
pub struct SubjectOutcome {
    pub term: String,
    pub memories: usize,
    /// Absent when the proposal was refused, or when nothing was generated for this subject.
    pub sentences: Option<usize>,
    /// Why it was refused, verbatim from the validator.
    pub rejected: Option<String>,
    pub stored: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SynthesisReport {
    /// Subjects that already carry prose describing their current memory set.
    pub already_current: usize,
    /// Subjects with no prose, or whose prose describes a memory set that has since changed.
    pub missing: usize,
    pub generated: usize,
    pub refused: usize,
    pub outcomes: Vec<SubjectOutcome>,
}

/// A subject that needs prose: its term, and the memory set the prose must describe.
type PendingSubject = (String, Vec<uuid::Uuid>);

/// Subjects whose stored prose does not describe their current memory set.
///
/// `subject_synthesis` returns `None` the moment the set changes, so "stale" and "absent" are the
/// same state here — which is the design: a page with no paragraph asserts nothing and cannot be
/// wrong, while a page with a paragraph about a memory set that no longer exists can be.
fn subjects_needing_prose(ledger: &EventLedger) -> Result<(Vec<PendingSubject>, usize)> {
    let memories = ledger.current_project_memories()?;
    let vectors: std::collections::HashMap<uuid::Uuid, Vec<f32>> = ledger
        .current_memory_vectors()
        .unwrap_or_default()
        .into_iter()
        .collect();
    if vectors.is_empty() {
        anyhow::bail!(
            "subjects are derived from embedding tightness, and this project has no memory \
             vectors yet — nothing can be synthesised until the backfill has run"
        );
    }
    let inputs: Vec<SubjectInput<'_>> = memories
        .iter()
        .map(|memory| SubjectInput {
            memory_id: memory.id,
            title: &memory.title,
        })
        .collect();

    let mut needing = Vec::new();
    let mut current = 0;
    for subject in derive_subjects(&inputs, &vectors)
        .into_iter()
        .take(MAX_SUBJECTS)
    {
        match ledger.subject_synthesis(&subject.term, &subject.memory_ids) {
            Ok(Some(_)) => current += 1,
            _ => needing.push((subject.term, subject.memory_ids)),
        }
    }
    Ok((needing, current))
}

/// Report what has prose and what does not, without calling the provider.
pub fn survey(ledger: &EventLedger) -> Result<SynthesisReport> {
    let (needing, already_current) = subjects_needing_prose(ledger)?;
    Ok(SynthesisReport {
        already_current,
        missing: needing.len(),
        outcomes: needing
            .into_iter()
            .map(|(term, ids)| SubjectOutcome {
                term,
                memories: ids.len(),
                sentences: None,
                rejected: None,
                stored: false,
            })
            .collect(),
        ..SynthesisReport::default()
    })
}

/// Draft, validate and store prose for up to `limit` subjects that lack it.
pub async fn generate(
    ledger: &EventLedger,
    provider: &dyn MergeProvider,
    limit: usize,
    now: time::OffsetDateTime,
) -> Result<SynthesisReport> {
    let (needing, already_current) = subjects_needing_prose(ledger)?;
    let mut report = SynthesisReport {
        already_current,
        missing: needing.len(),
        ..SynthesisReport::default()
    };
    let memories = ledger.current_project_memories()?;
    let by_id: std::collections::HashMap<uuid::Uuid, &brain_domain::MemoryRecord> =
        memories.iter().map(|memory| (memory.id, memory)).collect();

    for (term, memory_ids) in needing.into_iter().take(limit) {
        let instruction = instruction_for(&term, &memory_ids, &by_id);
        // A provider failure is this subject's failure, not the run's — the same lesson
        // `brain revise` learned when one timeout discarded seven completed merges.
        let body = match provider.merge(&instruction).await {
            Ok(body) => body,
            Err(error) => {
                report.refused += 1;
                report.outcomes.push(SubjectOutcome {
                    term,
                    memories: memory_ids.len(),
                    sentences: None,
                    rejected: Some(format!("provider call failed: {error}")),
                    stored: false,
                });
                continue;
            }
        };

        let checked = brain_context::parse_synthesis_response(&body)
            .map_err(|error| error.to_string())
            .and_then(|proposed| {
                brain_context::validate_synthesis(&proposed, &memory_ids)
                    .map_err(|rejection| rejection.to_string())
            });

        match checked {
            Ok(validated) => {
                let markdown =
                    validated.to_markdown(&|id| by_id.get(&id).map(|memory| memory.title.clone()));
                ledger.store_subject_synthesis(&term, &memory_ids, &markdown, now)?;
                report.generated += 1;
                report.outcomes.push(SubjectOutcome {
                    term,
                    memories: memory_ids.len(),
                    sentences: Some(validated.sentences.len()),
                    rejected: None,
                    stored: true,
                });
            }
            Err(rejection) => {
                report.refused += 1;
                report.outcomes.push(SubjectOutcome {
                    term,
                    memories: memory_ids.len(),
                    sentences: None,
                    rejected: Some(rejection),
                    stored: false,
                });
            }
        }
    }
    Ok(report)
}

/// The prompt: the shared instruction, then this subject's memories with their ids.
///
/// The ids have to be in the prompt because the instruction requires each sentence to cite one
/// verbatim, and the validator refuses any citation outside this set. Supplying the memories is the
/// caller's job — `synthesis_instruction` states the rules and nothing more, so the two halves can
/// be tested apart.
fn instruction_for(
    term: &str,
    memory_ids: &[uuid::Uuid],
    by_id: &std::collections::HashMap<uuid::Uuid, &brain_domain::MemoryRecord>,
) -> String {
    let mut prompt = brain_context::synthesis_instruction(term);
    prompt.push_str("\n\nMemories:\n");
    for id in memory_ids.iter().take(MAX_MEMORIES_PER_SUBJECT) {
        let Some(memory) = by_id.get(id) else {
            continue;
        };
        let excerpt: String = memory
            .content
            .chars()
            .take(MEMORY_EXCERPT_CHARACTERS)
            .collect();
        prompt.push_str(&format!(
            "\n- memory_id: {id}\n  title: {}\n  content: {}\n",
            memory.title,
            excerpt.replace('\n', " ")
        ));
    }
    prompt
}

pub fn render(report: &SynthesisReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "  {} subject(s) already describe their current memory set · {} need prose\n",
        report.already_current, report.missing
    ));
    if report.generated > 0 || report.refused > 0 {
        out.push_str(&format!(
            "  {} written · {} refused\n",
            report.generated, report.refused
        ));
    }
    if report.outcomes.is_empty() {
        return out;
    }
    out.push('\n');
    for outcome in &report.outcomes {
        match (&outcome.rejected, outcome.sentences) {
            (Some(reason), _) => {
                out.push_str(&format!(
                    "  REFUSED  {} ({} memories)\n           {}\n",
                    outcome.term,
                    outcome.memories,
                    reason.chars().take(140).collect::<String>()
                ));
            }
            (None, Some(sentences)) => out.push_str(&format!(
                "  written  {} — {} sentence(s) over {} memories\n",
                outcome.term, sentences, outcome.memories
            )),
            (None, None) => out.push_str(&format!(
                "  needs prose  {} ({} memories)\n",
                outcome.term, outcome.memories
            )),
        }
    }
    if report.generated > 0 {
        out.push_str("\n  The projector picks these up on its next pass.\n");
    }
    out
}
