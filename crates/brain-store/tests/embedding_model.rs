//! Behaviour of the real model, not of the code around it.
//!
//! Ignored by default because it needs an 87 MB checkpoint that is not vendored. Fetch it with:
//!
//! ```text
//! MODELS=~/AgentBrain/models/all-MiniLM-L6-v2
//! mkdir -p "$MODELS"
//! BASE=https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main
//! for f in config.json tokenizer.json model.safetensors; do curl -sL -o "$MODELS/$f" "$BASE/$f"; done
//! cargo test -p brain-store --test embedding_model -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use brain_store::{EMBEDDING_DIMENSIONS, Embedder, cosine_similarity, default_model_dir};

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn related_text_scores_above_unrelated_text() {
    // This is the whole reason the module exists. The pair below is the shape of a
    // `single-session-preference` question — the LongMemEval category keyword search scores
    // 63.3% on — and the two sentences share essentially no vocabulary, so BM25 ranks them
    // near-identically. If the margin here is not clear, mean pooling or normalisation is
    // wrong, and every vector produced would be plausible and useless.
    let Some(embedder) = load() else {
        panic!("model not installed; see the module comment for the download");
    };

    let query = "what do I usually prefer for deployment";
    let related = "I always ship straight to production on Fridays, never staged";
    let unrelated = "the cat sat on a warm windowsill in the afternoon";

    let q = embedder.embed(query).expect("embed query");
    assert_eq!(q.len(), EMBEDDING_DIMENSIONS);

    let near = cosine_similarity(&q, &embedder.embed(related).expect("embed related"));
    let far = cosine_similarity(&q, &embedder.embed(unrelated).expect("embed unrelated"));

    println!("  related   {near:.4}");
    println!("  unrelated {far:.4}");
    assert!(
        near - far > 0.10,
        "semantic ranking is not working: related {near:.4} vs unrelated {far:.4}"
    );
}

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn vectors_are_unit_length_so_cosine_is_a_dot_product() {
    // Storage and search both assume this. If normalisation regressed, similarity would carry
    // each vector's magnitude and longer text would simply score higher.
    let Some(embedder) = load() else {
        panic!("model not installed");
    };
    for text in [
        "short",
        "a considerably longer sentence with more tokens in it",
    ] {
        let v = embedder.embed(text).expect("embed");
        let length: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (length - 1.0).abs() < 1e-3,
            "vector for {text:?} has length {length}, expected 1"
        );
    }
}

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn an_oversized_input_is_truncated_rather_than_refused() {
    // One captured event reached 3.7 MB. Embedding must bound its own cost: without truncation
    // a single event would take as long as thousands of ordinary ones.
    let Some(embedder) = load() else {
        panic!("model not installed");
    };
    let huge = "the deployment pipeline rebuilt and restarted the service. ".repeat(4_000);
    let started = std::time::Instant::now();
    let v = embedder.embed(&huge).expect("embed oversized input");
    println!("  {} chars embedded in {:?}", huge.len(), started.elapsed());
    assert_eq!(v.len(), EMBEDDING_DIMENSIONS);
}

fn load() -> Option<Embedder> {
    // `default_model_dir` takes the brain home, not the user home.
    let brain_home = std::env::var("BRAIN_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .ok()?;
            Some(PathBuf::from(home).join("AgentBrain"))
        })?;
    Embedder::load_if_available(&default_model_dir(&brain_home))
}
