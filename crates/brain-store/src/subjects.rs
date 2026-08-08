//! Deciding what a subject is, without a model asserting one.
//!
//! The vault groups memories by *kind* — decision, fact, procedure — which is navigation, not
//! accumulation. Nothing compounds per subject: there is no page for "the deploy pipeline" that
//! gets richer as work continues, so nine thousand memories never become a wiki.
//!
//! The hard part was never rendering the page. It was deciding what a subject *is*, because a
//! subject an LLM invented is an unsourced claim sitting in a vault whose entire property is that
//! nothing in it is unsourced.
//!
//! # The discriminator, and the one that failed
//!
//! Candidate terms are easy — strip stopwords from memory titles and count. On the live corpus
//! that yields 6,683 terms in three or more memories, mixed indiscriminately: `discord`,
//! `pipeline` and `architecture` alongside `fix`, `via` and `only`. Frequency cannot separate them.
//!
//! **Evidence cohesion was tried first and does not work.** The intuition was that memories about
//! one subject would cite overlapping events. Measured over 2,030 memories, `add` scored 0.018
//! while `consolidation` scored 0.004 and `retrieval` 0.005 — noise beat every real subject. The
//! reason is worth keeping: memories sharing evidence were written from the same *bounded
//! consolidation job*, so co-citation measures temporal batching, not subject affinity. Two
//! memories about deployment written a month apart cite nothing in common. It is the right signal
//! for wikilinks, where it is already used, and the wrong one here.
//!
//! **Embedding tightness works.** A real subject's memories sit measurably closer together than two
//! memories drawn at random. Measured against a per-project random-pair baseline of 0.220:
//!
//! | term | lift | | term | lift |
//! |---|---|---|---|---|
//! | `vault` | +0.261 | | `fix` | +0.114 |
//! | `consolidation` | +0.215 | | `session` | +0.084 |
//! | `deploy` | +0.191 | | `add` | +0.014 |
//! | `backup` | +0.188 | | `via` | +0.006 |
//! | `retrieval` | +0.166 | | `only` | −0.012 |
//!
//! [`MINIMUM_LIFT`] sits between them. The baseline is computed per project rather than fixed,
//! because a narrow corpus is uniformly more similar than a broad one and a constant would mean
//! something different in each.
//!
//! # Exactness and determinism
//!
//! Mean pairwise cosine is computed in closed form rather than sampled. Stored vectors are
//! L2-normalised, so for a set of `n` of them the mean cosine over all pairs is exactly
//! `(‖Σv‖² − n) / (n(n−1))` — O(n) instead of O(n²), and *exact*, which sampling is not. That
//! matters beyond speed: the vault's generation is content-addressed, so a sampled score would
//! republish the whole vault every time the sample changed.

use std::collections::HashMap;

/// Fewest memories a term needs before it can be a subject.
///
/// Below this the tightness figure is noise — three memories that happen to share a word tell you
/// nothing about whether the word names a subject.
pub const MINIMUM_MEMORIES: usize = 5;

/// How much tighter than random a subject's memories must be.
///
/// Measured: real subjects land between +0.166 and +0.261 on the live corpus, filler words between
/// −0.012 and +0.114. This sits in the gap, nearer the noise floor than the signal, because
/// missing a real subject costs a page nobody sees and inventing one costs the vault's credibility.
pub const MINIMUM_LIFT: f32 = 0.15;

/// Most subject pages published per project.
///
/// A vault with a thousand subject pages is not navigable, and the tail of any such ranking is
/// terms that scraped past the threshold. The count is always reported so a truncated set never
/// implies the corpus holds fewer subjects than it does.
pub const MAX_SUBJECTS: usize = 150;

/// Shortest term worth considering.
const MINIMUM_TERM_CHARACTERS: usize = 3;

/// Words that carry no subject on their own.
///
/// Deliberately short. The tightness test is what removes `fix`, `via` and `only`; this list only
/// removes words too common to be worth *measuring*, which keeps the expensive filter honest
/// rather than doing its job for it in a hand-tuned list nobody can justify.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "was", "were", "are", "not",
    "but", "its", "it", "has", "have", "had", "will", "would", "can", "could", "should", "when",
    "then", "than", "them", "they", "their", "there", "which", "what", "who", "why", "how", "all",
    "any", "one", "two", "per", "via", "out", "off", "now", "new", "old", "use", "used", "uses",
    "get", "gets", "got", "set", "sets", "you", "your", "our",
];

/// A recurring subject, and the memories that mention it.
#[derive(Clone, Debug)]
pub struct Subject {
    pub term: String,
    /// Memory ids, in the order they were given.
    pub memory_ids: Vec<uuid::Uuid>,
    /// Mean pairwise cosine among this subject's memories.
    pub tightness: f32,
    /// How far above the project's random-pair baseline that sits.
    pub lift: f32,
}

/// One memory, as this module needs it.
pub struct SubjectInput<'a> {
    pub memory_id: uuid::Uuid,
    pub title: &'a str,
}

/// Derive the subjects of a memory set.
///
/// `vectors` need not cover every memory: a memory with no vector simply cannot contribute to a
/// tightness score, and a term whose memories are mostly unembedded will not reach the threshold.
/// That is the correct behaviour on a brain mid-backfill — fewer subject pages, never wrong ones.
pub fn derive_subjects(
    memories: &[SubjectInput<'_>],
    vectors: &HashMap<uuid::Uuid, Vec<f32>>,
) -> Vec<Subject> {
    let baseline = mean_pairwise_cosine(&vectors.values().collect::<Vec<_>>());
    let Some(baseline) = baseline else {
        return Vec::new();
    };

    let mut by_term: HashMap<String, Vec<uuid::Uuid>> = HashMap::new();
    for memory in memories {
        for term in terms(memory.title) {
            let ids = by_term.entry(term).or_default();
            // A title mentioning a word twice is one memory, not two.
            if !ids.contains(&memory.memory_id) {
                ids.push(memory.memory_id);
            }
        }
    }

    let mut subjects: Vec<Subject> = by_term
        .into_iter()
        .filter(|(_, ids)| ids.len() >= MINIMUM_MEMORIES)
        .filter_map(|(term, ids)| {
            let present: Vec<&Vec<f32>> = ids.iter().filter_map(|id| vectors.get(id)).collect();
            if present.len() < MINIMUM_MEMORIES {
                return None;
            }
            let tightness = mean_pairwise_cosine(&present)?;
            let lift = tightness - baseline;
            (lift >= MINIMUM_LIFT).then_some(Subject {
                term,
                memory_ids: ids,
                tightness,
                lift,
            })
        })
        .collect();

    // Rank by lift weighted by how much the subject covers, so a tight term spanning forty
    // memories outranks an equally tight one spanning five. Ties break on the term itself, which
    // keeps the published set — and therefore the vault's generation hash — stable across runs.
    subjects.sort_by(|left, right| {
        weight(right)
            .total_cmp(&weight(left))
            .then_with(|| left.term.cmp(&right.term))
    });
    subjects.truncate(MAX_SUBJECTS);
    subjects
}

fn weight(subject: &Subject) -> f32 {
    subject.lift * (subject.memory_ids.len() as f32).ln()
}

/// Mean cosine over every pair, in closed form.
///
/// Stored vectors are L2-normalised, so `Σᵢ Σⱼ vᵢ·vⱼ = ‖Σv‖²`. Subtracting the `n` self-pairs and
/// dividing by `n(n−1)` gives the mean over distinct ordered pairs, which equals the mean over
/// unordered ones. Exact, O(n), and independent of any sampling.
fn mean_pairwise_cosine(vectors: &[&Vec<f32>]) -> Option<f32> {
    let count = vectors.len();
    if count < 2 {
        return None;
    }
    let width = vectors[0].len();
    let mut sum = vec![0.0_f32; width];
    for vector in vectors {
        if vector.len() != width {
            return None;
        }
        for (slot, value) in sum.iter_mut().zip(vector.iter()) {
            *slot += value;
        }
    }
    let squared_norm: f32 = sum.iter().map(|value| value * value).sum();
    let n = count as f32;
    Some((squared_norm - n) / (n * (n - 1.0)))
}

/// Candidate terms from one title.
fn terms(title: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in title
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_lowercase)
    {
        if word.chars().count() < MINIMUM_TERM_CHARACTERS
            || STOPWORDS.contains(&word.as_str())
            || word.chars().all(|character| character.is_numeric())
        {
            continue;
        }
        found.push(word);
    }
    found
}
