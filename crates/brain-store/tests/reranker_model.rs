//! The cross-encoder, against the real checkpoint.
//!
//! Ignored by default because it needs the 86 MB model. Fetch it with:
//!
//! ```text
//! MODELS=~/AgentBrain/models/ms-marco-MiniLM-L6-v2
//! mkdir -p "$MODELS"
//! BASE=https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/resolve/main
//! for f in config.json tokenizer.json model.safetensors; do curl -sL -o "$MODELS/$f" "$BASE/$f"; done
//! cargo test -p brain-store --test reranker_model -- --ignored --nocapture
//! ```
//!
//! **These tests pin a limitation as firmly as they pin a capability.** The model was adopted to
//! fix ranking and it fixes one half of it; writing only the half that passes would leave the other
//! half to be rediscovered later as a surprise.

use std::path::PathBuf;

use brain_store::Reranker;

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see the module comment"]
fn the_head_is_wired_correctly() {
    // candle ships no sequence-classification head, so ours loads the checkpoint's own pooler and
    // classifier by hand — and a head that is subtly wrong produces plausible numbers rather than
    // an error. This is the model card's published pair, which is the only way to tell the
    // difference. Reference scores are 8.607 and -4.320; ours separate wider, which is a different
    // upload of the same architecture, not a different wiring.
    let reranker = Reranker::load(&model_dir()).expect("load reranker");
    let scores = reranker
        .scores(
            "How many people live in Berlin?",
            &[
                "Berlin has a population of 3,520,031 registered inhabitants in an area of 891.82 square kilometers.",
                "New York City is famous for the Metropolitan Museum of Art.",
            ],
        )
        .expect("score");
    println!("  relevant   : {:.3} (model card: 8.607)", scores[0]);
    println!("  irrelevant : {:.3} (model card: -4.320)", scores[1]);
    assert!(
        scores[0] > 7.0 && scores[0] < 10.0,
        "the relevant pair must land near the published 8.607, got {:.3}",
        scores[0]
    );
    assert!(scores[1] < -3.0, "the irrelevant pair must score negative");
}

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see the module comment"]
fn a_factual_question_separates_its_answer_from_a_merely_on_topic_neighbour() {
    // This is what the stage is for, and it does it well: three-way separation with the answer
    // clear of a neighbour that shares the subject but does not answer.
    let reranker = Reranker::load(&model_dir()).expect("load reranker");
    let scores = reranker
        .scores(
            "why are the binaries statically linked",
            &[
                "The binaries are built with +crt-static because the dynamic VC++ runtime is not on the DLL search path under Task Scheduler.",
                "Static linking makes each binary larger but self-contained.",
                "canines bark loudly whenever strangers approach a fence",
            ],
        )
        .expect("score");
    println!(
        "  answer {:.3} | on-topic {:.3} | unrelated {:.3}",
        scores[0], scores[1], scores[2]
    );
    assert!(
        scores[0] > scores[1] && scores[1] > scores[2],
        "measured 6.230 / 2.365 / -11.348; got {scores:?}"
    );
}

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see the module comment"]
fn a_vocabulary_gap_is_not_bridged_and_this_is_the_stage_s_real_limit() {
    // **The finding this stage must not be trusted past.**
    //
    // Holding the distractor constant and rewriting a single word of the answer — `ship` to
    // `deploy`, same claim, same meaning — moved its score from -11.131 to -0.913. Ten points, the
    // model's entire useful range, bought with vocabulary alone. Without that bridge the answer
    // scores below an unrelated sentence about dogs (-11.272) and loses to a distractor that merely
    // shares the subject.
    //
    // The bi-encoder fails the same four cases in the same direction (0.353 / 0.498 / 0.493 /
    // 0.618), so this is not a weakness re-ranking corrects — it is the same blind spot at a wider
    // scale. `single-session-preference`, the category hybrid retrieval was built for, is made
    // of exactly this shape, and nothing here helps it. The fix for a vocabulary gap is query
    // expansion, not a second scoring model.
    let reranker = Reranker::load(&model_dir()).expect("load reranker");
    let query = "what do I usually prefer when deploying";
    let distractor =
        "deployment takes about four minutes end to end, and the build is the slow part";

    let without_bridge = reranker
        .scores(
            query,
            &[
                "I always ship straight to production on Fridays, never through staging",
                distractor,
            ],
        )
        .expect("score");
    let with_bridge = reranker
        .scores(
            query,
            &[
                "I always deploy straight to production on Fridays, never through staging",
                distractor,
            ],
        )
        .expect("score");

    println!(
        "  no bridge : answer {:.3} vs distractor {:.3}",
        without_bridge[0], without_bridge[1]
    );
    println!(
        "  bridged   : answer {:.3} vs distractor {:.3}",
        with_bridge[0], with_bridge[1]
    );

    assert!(
        without_bridge[0] < without_bridge[1],
        "documenting the limit: without shared vocabulary the answer loses. If this ever starts \
         passing the model changed, and the query-expansion argument should be revisited"
    );
    assert!(
        with_bridge[0] > with_bridge[1],
        "with the bridge it wins, which is what isolates vocabulary as the variable"
    );
    assert!(
        with_bridge[0] - without_bridge[0] > 5.0,
        "one word should be worth most of the model's range: measured 10.2"
    );
}

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see the module comment"]
fn scoring_cost_is_measured_rather_than_assumed() {
    // `RERANK_DEPTH` is chosen from this number. `HOOK_HARD_TIMEOUT` was a reasonable number that
    // went silently wrong as the corpus grew; a re-rank window picked by feel would be the same
    // mistake with a different constant.
    let start = std::time::Instant::now();
    let reranker = Reranker::load(&model_dir()).expect("load reranker");
    println!("  load        : {:?}", start.elapsed());

    let document = format!(
        "I always ship straight to production. {}",
        "padding text to a realistic length. ".repeat(20)
    );
    for count in [1_usize, 8, 16, 32] {
        let documents: Vec<&str> = std::iter::repeat_n(document.as_str(), count).collect();
        let start = std::time::Instant::now();
        let scores = reranker
            .scores("what do I prefer", &documents)
            .expect("score");
        let elapsed = start.elapsed();
        assert_eq!(scores.len(), count);
        println!(
            "  {count:>2} documents: {elapsed:?} ({:?} each)",
            elapsed / u32::try_from(count).expect("count")
        );
    }
}

#[test]
#[ignore = "needs the ms-marco-MiniLM-L6-v2 checkpoint; see the module comment"]
fn a_document_past_the_position_limit_scores_rather_than_failing() {
    // Truncation is `OnlySecond`, so an enormous document loses its tail and the question survives
    // intact. If it ever became `LongestFirst` the question would start losing tokens instead,
    // which changes what was asked rather than what was read — and would do so silently.
    let reranker = Reranker::load(&model_dir()).expect("load reranker");
    let enormous = "unrelated filler. ".repeat(4000);
    let scores = reranker
        .scores(
            "why are the binaries statically linked",
            &[enormous.as_str()],
        )
        .expect("a document past the position limit must score, not fail");
    assert_eq!(scores.len(), 1);
    assert!(
        scores[0] < 0.0,
        "filler is not an answer, so it should score negative; got {:.3}",
        scores[0]
    );
}

fn model_dir() -> PathBuf {
    std::env::var("BRAIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();
            PathBuf::from(home).join("AgentBrain")
        })
        .join("models")
        .join("ms-marco-MiniLM-L6-v2")
}
