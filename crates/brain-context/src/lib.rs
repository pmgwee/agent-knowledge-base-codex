#![forbid(unsafe_code)]

mod authority;
mod citations;
mod compiler;
mod glm;
mod live_state;
mod llm;
mod providers;
mod query;
mod retrieval;
mod supersession;
mod token_budget;

pub use authority::authority_rank;
pub use citations::Citation;
pub use compiler::{CompiledContext, ContextCompiler, ContextEvidence};
pub use glm::{GlmClient, GlmConfig, parse_glm_chat_response};
pub use live_state::LiveState;
pub use llm::{
    ConsolidationLlm, EvidencePacket, ProposedMemory, ProposedMemoryBatch, RedactedEvidence,
    validate_proposed_batch,
};
pub use providers::{
    CodeGraphConfig, ContextProvider, GuardedProviderResult, LlmWikiConfig, ProviderConfig,
    ProviderGuard, ProviderResult, ProviderStatus, retrieve_provider_results,
};
pub use query::{ContextQuery, HARD_MAX_TOKENS, NORMAL_STARTUP_TOKENS, RetrievalQuery};
pub use retrieval::{RankedCandidate, RetrievalEngine, ScoreComponents};
pub use supersession::{MemoryConflict, ResolvedMemorySet, resolve_candidates};
pub use token_budget::token_count;
