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

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn steady_state_throughput_is_measured_not_assumed() {
    // The first embedding of a process pays for allocator warmup and lazily-initialised kernels.
    // Sizing a backfill from that number would overstate the cost by an order of magnitude, so
    // measure a warm loop instead — and measure it on text the length of real captured turns,
    // since cost grows with sequence length.
    let Some(embedder) = load() else {
        panic!("model not installed");
    };
    let turn = "I refactored the consolidation worker so a provider outage defers the job \
                instead of consuming one of its five attempts, then redeployed and watched \
                the queue drain.";

    let first = std::time::Instant::now();
    embedder.embed(turn).expect("cold embed");
    let cold = first.elapsed();

    let n = 50;
    let started = std::time::Instant::now();
    for _ in 0..n {
        embedder.embed(turn).expect("warm embed");
    }
    let per = started.elapsed() / n;

    println!("  cold first call : {cold:?}");
    println!("  warm per call   : {per:?}");
    println!(
        "  implied rate    : {:.0} embeddings/sec",
        1.0 / per.as_secs_f64()
    );
    println!(
        "  6,761 memories  : {:.1} min",
        6_761.0 * per.as_secs_f64() / 60.0
    );
    println!(
        "  246,750 turns   : {:.1} min",
        246_750.0 * per.as_secs_f64() / 60.0
    );
    assert!(per.as_millis() < 500, "unusably slow at {per:?}");
}

#[test]
#[ignore = "needs the all-MiniLM-L6-v2 checkpoint; see the module comment"]
fn batching_amortises_the_per_call_cost() {
    // Measured at ~1.2x, not the several-fold win batching usually gives: the cost here is
    // the matmuls, not per-call overhead, because candle's CPU backend has no optimised BLAS.
    // The bar is therefore "not slower", which still catches a regression that made batching
    // pointless — and the printed figure is the number any backfill sizing must use, rather
    // than an assumed speedup that would have been wrong by a factor of five.
    let Some(embedder) = load() else {
        panic!("model not installed");
    };
    let turn = "I refactored the consolidation worker so a provider outage defers the job \
                instead of consuming one of its five attempts, then redeployed.";
    let batch: Vec<&str> = std::iter::repeat_n(turn, 32).collect();

    embedder.embed(turn).expect("warm up");

    let started = std::time::Instant::now();
    for text in &batch {
        embedder.embed(text).expect("one at a time");
    }
    let looped = started.elapsed();

    let started = std::time::Instant::now();
    let vectors = embedder.embed_batch(&batch).expect("batched");
    let batched = started.elapsed();

    assert_eq!(vectors.len(), batch.len());
    let speedup = looped.as_secs_f64() / batched.as_secs_f64();
    println!("  32 one-at-a-time : {looped:?}");
    println!("  32 batched       : {batched:?}");
    println!("  speedup          : {speedup:.1}x");
    println!(
        "  246,750 turns    : {:.0} min batched",
        246_750.0 * (batched.as_secs_f64() / 32.0) / 60.0
    );
    assert!(
        speedup > 1.0,
        "batching is now slower than looping: {speedup:.2}x"
    );
}
