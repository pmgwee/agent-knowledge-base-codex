#![forbid(unsafe_code)]

mod access;
mod backup;
mod basic_memory;
mod blob;
mod catalog;
mod context_metrics;
mod cursor;
mod embedding;
mod evict;
mod jobs;
mod ledger;
mod markdown;
mod memory;
mod migrations;
mod notes;
mod provider_cache;
mod rerank;
mod search;
mod segment;
mod subjects;
mod synthesis_store;
mod tombstone;
mod upgrade;
mod vector;

pub use access::MemoryAccess;
pub use backup::{
    BACKUP_FORMAT_VERSION, BackupInventory, BackupManager, BackupReport, InventoryFile,
    InventoryKind, RecoveryDrillReport, RestoreReport, RetentionPolicy, RetentionReport,
    VerificationReport,
};
pub use basic_memory::{
    BASIC_MEMORY_PINNED_VERSION, BasicMemoryCli, BasicMemoryIndexer, BasicMemoryReport,
    BasicMemoryState, ProcessBasicMemoryCli,
};
pub use blob::{BlobRecord, BlobStore};
pub use catalog::{CatalogEvent, SegmentCatalog};
pub use context_metrics::{ContextDelivery, ContextDeliverySummary};
pub use embedding::{
    EMBEDDING_DIMENSIONS, Embedder, MAX_INPUT_TOKENS, cosine_similarity, decode_vector,
    default_model_dir, encode_vector, shared_embedder,
};
pub use evict::{EvictionCandidate, EvictionGate, EvictionPlan, MINIMUM_OBSERVATION, QUIET_FOR};
pub use jobs::{
    ConsolidationJob, ConsolidationQueue, ConsolidationReason, JobStatus, MAX_JOB_EVENTS,
    MAX_JOB_PAYLOAD_BYTES, RedactionManifestEntry,
};
pub use ledger::{AppendResult, EventLedger, StoredEvent};
pub use markdown::{
    MarkdownProjector, ProjectionReport, ProjectionVerification, project_vault_root,
};
pub use memory::GlobalPreferenceStore;
pub use provider_cache::{ProviderCacheEntry, ProviderCacheStore};
pub use rerank::{MAX_PAIR_TOKENS, Reranker, default_reranker_dir, shared_reranker};
pub use search::{
    RetrievalConfiguration, SearchHit, SearchQuery, SearchSource, SearchSourceFilter, TimeRange,
    retrieval_configuration,
};
pub use segment::{
    SEGMENT_FORMAT_VERSION, SegmentManifest, SegmentSealReport, SegmentStore, should_seal,
};
pub use subjects::{
    MAX_SUBJECTS, MINIMUM_LIFT, MINIMUM_MEMORIES, Subject, SubjectInput, derive_subjects,
};
pub use synthesis_store::{StoredSynthesis, memory_set_hash};
pub use tombstone::Tombstone;
pub use upgrade::{UpgradeIssue, UpgradeManager, UpgradeReport, UpgradeStageReport};
pub use vector::{
    EMBEDDING_MODEL, EventVectorHit, PendingEmbedding, PendingEventEmbedding, VectorHit,
};
