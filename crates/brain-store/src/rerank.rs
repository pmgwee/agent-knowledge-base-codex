//! Cross-encoder re-ranking, in-process.
//!
//! Fusion decides *reach*: whether the answering document is in the candidate set at all. It does
//! not decide *responsiveness*, and the two are different problems that a bi-encoder cannot tell
//! apart. `all-MiniLM-L6-v2` scores whether two texts are **alike**; measured against a merely
//! on-topic haystack, a turn about deployment *speed* scored 0.478 where the turn that actually
//! answered the question scored 0.353. Both are about deployment. Only one is an answer.
//!
//! A cross-encoder does the thing a bi-encoder structurally cannot: it puts the query and the
//! document in **one sequence** and runs attention across the boundary, so the score can depend on
//! how the two relate rather than on how each looks alone. That is an architectural difference, not
//! a size difference — this checkpoint is the same 6-layer, 384-hidden BERT as our embedder, at
//! 86 MB. `Qwen3-Reranker-0.6B` was considered and rejected: 28 layers of 1024 at 1.1 GB on disk
//! and ~2.3 GB resident at our `F32` loader, inside a service that runs permanently, for a model
//! that scores by reading yes/no token logits out of a causal LM under a chat template — three new
//! ways to be subtly, silently wrong. If this checkpoint proves too weak the next rung is
//! `bge-reranker-base`, not a billion parameters.
//!
//! **The head is ours.** candle ships `BertModel` and `BertForMaskedLM` but no sequence
//! classification, so the pooler (`Linear(384,384)` + tanh over `[CLS]`) and the classifier
//! (`Linear(384,1)`) are loaded here directly from the checkpoint's own tensors.
//!
//! **Absence is not an error**, exactly as with the embedder. No checkpoint means no re-ranking and
//! the fused order stands — an optional stage may only ever reorder what retrieval already found.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::{Linear, Module, VarBuilder, linear};
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams, TruncationStrategy};

/// Longest (query, document) pair handed to the model, in tokens.
///
/// The checkpoint has 512 positions and truncation is `OnlySecond`, so a long document loses its
/// tail and the query is never cut. Truncating the query instead would change the question being
/// asked, which is a far worse failure than losing the end of a transcript turn.
pub const MAX_PAIR_TOKENS: usize = 512;

/// Where the re-ranking model lives when no path is given.
pub fn default_reranker_dir(brain_home: &Path) -> PathBuf {
    brain_home.join("models").join("ms-marco-MiniLM-L6-v2")
}

/// A loaded cross-encoder.
pub struct Reranker {
    model: BertModel,
    pooler: Linear,
    classifier: Linear,
    tokenizer: Tokenizer,
    device: Device,
}

impl Reranker {
    /// Load from a directory holding `config.json`, `tokenizer.json` and `model.safetensors`.
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_slice(
            &std::fs::read(dir.join("config.json")).context("read reranker config")?,
        )
        .context("parse reranker config")?;

        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|error| anyhow::anyhow!("read reranker tokenizer: {error}"))?;
        // Truncate the document, never the question.
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_PAIR_TOKENS,
                strategy: TruncationStrategy::OnlySecond,
                ..Default::default()
            }))
            .map_err(|error| anyhow::anyhow!("configure reranker truncation: {error}"))?;
        tokenizer.with_padding(Some(PaddingParams::default()));

        // Read rather than memory-map: mapping is candle's faster route but only through an
        // `unsafe` call, and this crate forbids unsafe outright. Same trade as the embedder.
        let weights =
            std::fs::read(dir.join("model.safetensors")).context("read reranker weights")?;
        let variables = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .context("load reranker weights")?;

        // `BertForSequenceClassification` stores the encoder under `bert.` and the classifier at
        // the root. The pooler lives inside `bert.` but candle's `BertModel` does not load it, so
        // both head tensors are taken directly.
        let model =
            BertModel::load(variables.pp("bert"), &config).context("load reranker encoder")?;
        let hidden = config.hidden_size;
        let pooler = linear(
            hidden,
            hidden,
            variables.pp("bert").pp("pooler").pp("dense"),
        )
        .context("load reranker pooler")?;
        let classifier =
            linear(hidden, 1, variables.pp("classifier")).context("load reranker classifier")?;

        Ok(Self {
            model,
            pooler,
            classifier,
            tokenizer,
            device,
        })
    }

    /// Load if the checkpoint is present, otherwise `None`.
    ///
    /// A present-but-unloadable model logs rather than returns, because at that point something is
    /// wrong and silence would hide it while ranking quietly stayed worse than it should be.
    pub fn load_if_available(dir: &Path) -> Option<Self> {
        if !dir.join("model.safetensors").is_file() {
            return None;
        }
        match Self::load(dir) {
            Ok(reranker) => Some(reranker),
            Err(error) => {
                tracing::warn!(
                    directory = %dir.display(),
                    %error,
                    "reranker present but unusable; keeping the fused order"
                );
                None
            }
        }
    }

    /// Score how well each document answers the query.
    ///
    /// Higher is more relevant. The checkpoint declares its activation as `Identity`, so the raw
    /// logit *is* the score — it is unbounded and frequently negative, which is fine for ordering
    /// and meaningless as a magnitude. Never compare one across queries.
    pub fn scores(&self, query: &str, documents: &[&str]) -> Result<Vec<f32>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_ids = Vec::with_capacity(documents.len());
        let mut all_types = Vec::with_capacity(documents.len());
        let mut all_mask = Vec::with_capacity(documents.len());
        for document in documents {
            let encoding = self
                .tokenizer
                .encode((query, *document), true)
                .map_err(|error| anyhow::anyhow!("tokenize for reranking: {error}"))?;
            let ids = encoding.get_ids().to_vec();
            anyhow::ensure!(!ids.is_empty(), "cannot rerank an empty pair");
            all_types.push(encoding.get_type_ids().to_vec());
            all_mask.push(encoding.get_attention_mask().to_vec());
            all_ids.push(ids);
        }

        let width = all_ids.iter().map(Vec::len).max().unwrap_or(0);
        for ((ids, types), mask) in all_ids
            .iter_mut()
            .zip(all_types.iter_mut())
            .zip(all_mask.iter_mut())
        {
            ids.resize(width, 0);
            types.resize(width, 0);
            // Zero here is what keeps padding out of attention, and so out of the score.
            mask.resize(width, 0);
        }

        let shape = (documents.len(), width);
        let input = Tensor::from_vec(all_ids.concat(), shape, &self.device)?;
        let token_types = Tensor::from_vec(all_types.concat(), shape, &self.device)?;
        let attention = Tensor::from_vec(all_mask.concat(), shape, &self.device)?;

        let hidden = self.model.forward(&input, &token_types, Some(&attention))?;
        // `BertForSequenceClassification` pools the `[CLS]` position only — not a mean over the
        // sequence. Mean-pooling here would silently produce a different model's answer.
        let cls = hidden.i((.., 0))?;
        let pooled = self.pooler.forward(&cls)?.tanh()?;
        let logits = self.classifier.forward(&pooled)?;
        Ok(logits.flatten_all()?.to_vec1::<f32>()?)
    }
}

/// Process-wide reranker, loaded once.
static SHARED: OnceLock<Option<Reranker>> = OnceLock::new();

/// The shared reranker for a brain home, or `None` when no checkpoint is installed.
pub fn shared_reranker(brain_home: &Path) -> Option<&'static Reranker> {
    SHARED
        .get_or_init(|| Reranker::load_if_available(&default_reranker_dir(brain_home)))
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_checkpoint_yields_no_reranker_rather_than_an_error() {
        // Ranking must keep working on a brain that never installed the model.
        let temp = tempfile::tempdir().expect("temp");
        assert!(Reranker::load_if_available(temp.path()).is_none());
    }
}
