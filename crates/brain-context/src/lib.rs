#![forbid(unsafe_code)]

mod authority;
mod compiler;
mod glm;
mod llm;
mod query;
mod retrieval;
mod supersession;
mod token_budget;

pub use authority::authority_rank;
pub use compiler::{CompiledContext, ContextCompiler, ContextEvidence};
pub use glm::{GlmClient, GlmConfig, parse_glm_chat_response};
pub use llm::{
    ConsolidationLlm, EvidencePacket, ProposedMemory, ProposedMemoryBatch, RedactedEvidence,
    validate_proposed_batch,
};
pub use query::{ContextQuery, HARD_MAX_TOKENS, NORMAL_STARTUP_TOKENS, RetrievalQuery};
pub use retrieval::{RankedCandidate, RetrievalEngine, ScoreComponents};
pub use supersession::{MemoryConflict, ResolvedMemorySet, resolve_candidates};
pub use token_budget::token_count;
