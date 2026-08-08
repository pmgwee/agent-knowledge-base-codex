//! What counts as a subject, and what must not.
//!
//! The discriminator is measured rather than asserted, and the measurement is in `subjects.rs`.
//! These tests pin the behaviour that measurement implies: tight clusters become subjects, terms
//! that merely recur do not, and the arithmetic is exact rather than sampled.

use std::collections::HashMap;

use brain_store::{MINIMUM_MEMORIES, SubjectInput, derive_subjects};

/// Build a unit vector pointing mostly along `axis`, with `spread` of noise on a shared axis.
///
/// Small spread means a tight cluster; large spread pulls everything toward a common direction and
/// makes each group indistinguishable from the corpus average, which is what a filler word looks
/// like in embedding space.
fn vector(axis: usize, jitter: usize, spread: f32) -> Vec<f32> {
    let mut v = [0.0_f32; 32];
    v[axis] = 1.0;
    v[16 + (jitter % 8)] = spread;
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / norm).collect()
}

#[test]
fn a_tight_cluster_becomes_a_subject_and_a_scattered_one_does_not() {
    // Two terms appear in the same number of memories. One names a real subject — its memories
    // point the same way — and the other is a filler word scattered across the corpus. Frequency
    // cannot tell them apart, which is the entire reason tightness is measured.
    let mut titles = Vec::new();
    let mut vectors = HashMap::new();
    let mut ids = Vec::new();

    for index in 0..8 {
        let id = uuid::Uuid::from_bytes([index as u8; 16]);
        ids.push(id);
        // Every title carries `scattered`; the first six also carry `deploy`.
        titles.push(if index < 6 {
            format!("deploy pipeline note {index} scattered")
        } else {
            format!("unrelated backup note {index} scattered")
        });
        // The `deploy` memories cluster on one axis; the others sit far away.
        vectors.insert(
            id,
            if index < 6 {
                vector(0, index, 0.05)
            } else {
                vector(index, index, 0.05)
            },
        );
    }

    let inputs: Vec<SubjectInput<'_>> = ids
        .iter()
        .zip(&titles)
        .map(|(id, title)| SubjectInput {
            memory_id: *id,
            title,
        })
        .collect();

    let subjects = derive_subjects(&inputs, &vectors);
    let found: Vec<&str> = subjects.iter().map(|s| s.term.as_str()).collect();

    assert!(
        found.contains(&"deploy"),
        "a term whose memories cluster is a subject, got {found:?}"
    );
    assert!(
        !found.contains(&"scattered"),
        "a term spread across the whole corpus is not a subject, got {found:?}"
    );
}

#[test]
fn a_term_in_too_few_memories_is_never_a_subject() {
    // Below the floor the tightness figure is noise: three memories sharing a word say nothing
    // about whether the word names anything.
    let mut vectors = HashMap::new();
    let mut ids = Vec::new();
    for index in 0..(MINIMUM_MEMORIES - 1) {
        let id = uuid::Uuid::from_bytes([index as u8; 16]);
        ids.push(id);
        vectors.insert(id, vector(0, index, 0.01));
    }
    let titles: Vec<String> = (0..ids.len())
        .map(|index| format!("rare subject mention {index}"))
        .collect();
    let inputs: Vec<SubjectInput<'_>> = ids
        .iter()
        .zip(&titles)
        .map(|(id, title)| SubjectInput {
            memory_id: *id,
            title,
        })
        .collect();

    assert!(
        derive_subjects(&inputs, &vectors).is_empty(),
        "four memories are not enough to name a subject"
    );
}

#[test]
fn a_brain_with_no_vectors_derives_no_subjects_rather_than_guessing() {
    // Subjects are derived from embeddings. Without them there is no way to tell `deploy` from
    // `fix`, and inventing pages would put unsourced structure into a vault whose whole property
    // is that nothing in it is unsourced.
    let titles: Vec<String> = (0..10)
        .map(|index| format!("deploy pipeline note {index}"))
        .collect();
    let ids: Vec<uuid::Uuid> = (0..10)
        .map(|index| uuid::Uuid::from_bytes([index as u8; 16]))
        .collect();
    let inputs: Vec<SubjectInput<'_>> = ids
        .iter()
        .zip(&titles)
        .map(|(id, title)| SubjectInput {
            memory_id: *id,
            title,
        })
        .collect();

    assert!(derive_subjects(&inputs, &HashMap::new()).is_empty());
}

#[test]
fn the_derivation_is_deterministic() {
    // The vault's generation is content-addressed, so a score that moved between runs would
    // republish everything each time. Sampling would do exactly that; the closed form does not.
    let mut vectors = HashMap::new();
    let ids: Vec<uuid::Uuid> = (0..12)
        .map(|index| uuid::Uuid::from_bytes([index as u8; 16]))
        .collect();
    for (index, id) in ids.iter().enumerate() {
        vectors.insert(*id, vector(if index < 7 { 0 } else { index }, index, 0.05));
    }
    let titles: Vec<String> = (0..12)
        .map(|index| {
            if index < 7 {
                format!("deploy note {index}")
            } else {
                format!("other note {index}")
            }
        })
        .collect();
    let inputs: Vec<SubjectInput<'_>> = ids
        .iter()
        .zip(&titles)
        .map(|(id, title)| SubjectInput {
            memory_id: *id,
            title,
        })
        .collect();

    let first = derive_subjects(&inputs, &vectors);
    let second = derive_subjects(&inputs, &vectors);
    assert_eq!(
        first.iter().map(|s| s.term.as_str()).collect::<Vec<_>>(),
        second.iter().map(|s| s.term.as_str()).collect::<Vec<_>>()
    );
    assert!(
        first
            .iter()
            .zip(&second)
            .all(|(a, b)| (a.tightness - b.tightness).abs() < f32::EPSILON),
        "the tightness score must be exact, not sampled"
    );
}
