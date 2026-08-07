//! Dense embeddings, in-process.
//!
//! Keyword search cannot answer a question that shares no words with its evidence. Measured on
//! LongMemEval-S, this ledger scores 90–100% on every question type except
//! `single-session-preference`, which sits at **63.3%** — the category where someone asks what
//! they usually prefer and the evidence says "I always ship straight to production", with barely
//! a token in common. That gap is the reason this module exists.
//!
//! `all-MiniLM-L6-v2` (384-dim) is the model because it is the one that produced the published
//! 95.2% R@5 on this exact benchmark. Matching it keeps our number comparable rather than merely
//! adjacent.
//!
//! It runs **in this process**. That was not a given: the binaries are built with
//! `+crt-static` — see `.cargo/config.toml`, and the Codex and Task Scheduler load failures that
//! forced it — and the usual route to local embeddings is ONNX Runtime, whose prebuilt libraries
//! link against the dynamic CRT. `candle` is pure Rust, so it links cleanly, and the resulting
//! binary was verified to import no `VCRUNTIME140`/`MSVCP` symbols at all. The reference
//! implementation this is measured against runs its embeddings in a separate Node process; this
//! needs no sidecar, no install step, and nothing to supervise.
//!
//! **Absence is not an error.** A brain with no model on disk searches by keyword exactly as it
//! did before. Every entry point returns `None` rather than failing, because a missing optional
//! index must never be able to break retrieval that already works.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::Tokenizer;

/// Vector width of `all-MiniLM-L6-v2`. Stored vectors are validated against it, since a model
/// swapped underneath a populated index would otherwise compare vectors of different meanings.
pub const EMBEDDING_DIMENSIONS: usize = 384;

/// Longest input handed to the model, in tokens.
///
/// The checkpoint's own limit is 512 positions; anything past that is truncated by the
/// tokenizer regardless. Naming it here makes the truncation a decision rather than a surprise,
/// and keeps latency predictable — cost grows with sequence length, and a 3.7 MB event would
/// otherwise take as long as thousands of ordinary ones.
pub const MAX_INPUT_TOKENS: usize = 512;

/// Where the model lives when no path is given.
pub fn default_model_dir(brain_home: &Path) -> PathBuf {
    brain_home.join("models").join("all-MiniLM-L6-v2")
}

/// A loaded sentence embedder.
///
/// Loading takes ~80 ms and encoding ~77 ms per sentence on CPU, so an instance is worth keeping
/// rather than rebuilding per call.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl Embedder {
    /// Load from a directory holding `config.json`, `tokenizer.json` and `model.safetensors`.
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_slice(
            &std::fs::read(dir.join("config.json")).context("read embedding model config")?,
        )
        .context("parse embedding model config")?;
        anyhow::ensure!(
            config.hidden_size == EMBEDDING_DIMENSIONS,
            "embedding model is {}-dimensional; this ledger stores {EMBEDDING_DIMENSIONS}",
            config.hidden_size
        );
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| anyhow::anyhow!("read embedding tokenizer: {error}"))?;
        // Read the weights rather than memory-mapping them. Mapping is the faster route and
        // candle offers it, but only through an `unsafe` call, and this crate forbids unsafe
        // outright. The cost is holding 87 MB while the model loads, once per process, against
        // a guarantee that covers every line in the crate — a trade worth making for a load
        // that already completes in about 80 ms.
        let weights =
            std::fs::read(dir.join("model.safetensors")).context("read embedding model weights")?;
        let variables = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .context("load embedding model weights")?;
        let model = BertModel::load(variables, &config).context("load embedding model")?;
        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Load if the model is present, otherwise `None`.
    ///
    /// A brain without the model is not broken; it is a brain that searches by keyword. This
    /// returns `None` for a missing directory and logs — rather than returns — a model that is
    /// present but unloadable, because at that point something is wrong and silence would hide
    /// it while retrieval quietly got worse.
    pub fn load_if_available(dir: &Path) -> Option<Self> {
        if !dir.join("model.safetensors").is_file() {
            return None;
        }
        match Self::load(dir) {
            Ok(embedder) => Some(embedder),
            Err(error) => {
                tracing::warn!(
                    directory = %dir.display(),
                    %error,
                    "embedding model present but unusable; searching by keyword only"
                );
                None
            }
        }
    }

    /// Embed one string into a unit-length vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed_batch(std::slice::from_ref(&text))?.remove(0))
    }

    /// Embed several strings in one forward pass.
    ///
    /// **Measured at 1.2x against the same work done one call at a time** — worth having, but
    /// far less than batching usually buys. The reason is that the cost here is not per-call
    /// overhead but the matmuls themselves: candle's CPU backend runs them without an optimised
    /// BLAS, so a batch of 32 does roughly 32 batches' worth of arithmetic. Padding to the
    /// longest member gives some of that back.
    ///
    /// The lever that would actually matter is parallelism across cores, or linking an
    /// optimised BLAS — neither of which this does today. Sizing a backfill from an assumed
    /// batching speedup would have been wrong by a factor of five.
    ///
    /// Sequences are padded to the longest in the batch, and the attention mask keeps the
    /// padding out of both the model and the pooling. Sorting by length before batching would
    /// waste less on padding still, but the caller owns ordering here and returning results in a
    /// different order than they were asked for is a far worse trap than some wasted compute.
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_ids = Vec::with_capacity(texts.len());
        let mut all_mask = Vec::with_capacity(texts.len());
        for text in texts {
            let encoding = self
                .tokenizer
                .encode(*text, true)
                .map_err(|error| anyhow::anyhow!("tokenize for embedding: {error}"))?;
            let mut ids = encoding.get_ids().to_vec();
            let mut mask = encoding.get_attention_mask().to_vec();
            ids.truncate(MAX_INPUT_TOKENS);
            mask.truncate(MAX_INPUT_TOKENS);
            anyhow::ensure!(!ids.is_empty(), "cannot embed an empty string");
            all_ids.push(ids);
            all_mask.push(mask);
        }

        let width = all_ids.iter().map(Vec::len).max().unwrap_or(0);
        for (ids, mask) in all_ids.iter_mut().zip(all_mask.iter_mut()) {
            ids.resize(width, 0);
            // Zero in the mask is what keeps the padding out of attention and out of the mean.
            mask.resize(width, 0);
        }

        let flat_ids: Vec<u32> = all_ids.concat();
        let flat_mask: Vec<u32> = all_mask.concat();
        let shape = (texts.len(), width);
        let input = Tensor::from_vec(flat_ids, shape, &self.device)?;
        let attention = Tensor::from_vec(flat_mask, shape, &self.device)?;
        let token_types = input.zeros_like()?;
        let hidden = self.model.forward(&input, &token_types, Some(&attention))?;

        let mask_f = attention.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = hidden
            .broadcast_mul(&mask_f)?
            .sum(1)?
            .broadcast_div(&mask_f.sum(1)?)?;
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(pooled.broadcast_div(&norm)?.to_vec2::<f32>()?)
    }

    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| anyhow::anyhow!("tokenize for embedding: {error}"))?;
        let mut ids = encoding.get_ids().to_vec();
        let mut mask = encoding.get_attention_mask().to_vec();
        ids.truncate(MAX_INPUT_TOKENS);
        mask.truncate(MAX_INPUT_TOKENS);
        anyhow::ensure!(!ids.is_empty(), "cannot embed an empty string");

        let input = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let attention = Tensor::new(mask.as_slice(), &self.device)?.unsqueeze(0)?;
        let token_types = input.zeros_like()?;
        let hidden = self.model.forward(&input, &token_types, Some(&attention))?;

        // Mean-pool over real tokens only. Averaging padding in pulls every short input toward
        // a common point, which produces vectors that exist and rank nothing.
        let mask_f = attention.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = hidden
            .broadcast_mul(&mask_f)?
            .sum(1)?
            .broadcast_div(&mask_f.sum(1)?)?;

        // Unit length, so cosine similarity is a dot product and stored vectors are directly
        // comparable without carrying their magnitudes around.
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        Ok(pooled.broadcast_div(&norm)?.squeeze(0)?.to_vec1::<f32>()?)
    }
}

/// Cosine similarity between two unit vectors.
///
/// A plain dot product, valid only because `embed` normalises. Mismatched widths score 0 rather
/// than panicking: that means a vector written by a different model, and scoring it zero
/// excludes it from results instead of taking down the search that found it.
pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return 0.0;
    }
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

/// Pack a vector for storage as a SQLite BLOB.
pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Unpack a stored vector, or `None` if the blob is not a whole number of `f32`s of the
/// expected width. A corrupt or foreign vector is skipped, never guessed at.
pub fn decode_vector(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() != EMBEDDING_DIMENSIONS * 4 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Process-wide embedder, loaded once.
static SHARED: OnceLock<Option<Embedder>> = OnceLock::new();

/// The shared embedder for a brain home, or `None` when no model is installed.
pub fn shared_embedder(brain_home: &Path) -> Option<&'static Embedder> {
    SHARED
        .get_or_init(|| Embedder::load_if_available(&default_model_dir(brain_home)))
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_survives_a_round_trip_through_storage() {
        let vector: Vec<f32> = (0..EMBEDDING_DIMENSIONS)
            .map(|i| (i as f32) / 1000.0 - 0.19)
            .collect();
        let decoded = decode_vector(&encode_vector(&vector)).expect("round trip");
        assert_eq!(decoded, vector);
    }

    #[test]
    fn a_vector_of_the_wrong_width_is_skipped_rather_than_guessed_at() {
        // A stored vector of another width means a different model wrote it. Reading it as if
        // it were ours would compare coordinates that do not correspond.
        assert!(decode_vector(&[0_u8; 16]).is_none());
        assert!(decode_vector(&[]).is_none());
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_of_unit_vectors_is_their_dot_product() {
        let a = [1.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &a), 1.0);
        assert_eq!(cosine_similarity(&a, &[0.0, 1.0, 0.0]), 0.0);
        assert_eq!(cosine_similarity(&a, &[-1.0, 0.0, 0.0]), -1.0);
    }

    #[test]
    fn a_missing_model_yields_no_embedder_rather_than_an_error() {
        // Keyword search must keep working on a brain that never installed a model.
        let temp = tempfile::tempdir().expect("temp");
        assert!(Embedder::load_if_available(temp.path()).is_none());
    }
}
