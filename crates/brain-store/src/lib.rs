#![forbid(unsafe_code)]

/// The predicate that means "this row is the memory's current claim", for a `memory_versions v`.
///
/// Two conditions, and for most of this system's history only one of them mattered. Every memory
/// had exactly one version, so `v.status = 'current'` and "the memory's latest version" selected
/// the same rows and the difference was unobservable. The first supersession made them diverge in
/// **both** directions at once:
///
/// - A **retired** memory keeps its original `current` version and gains a `superseded` one, so a
///   status-only filter still returns the claim that was withdrawn.
/// - A **keeper** gains a second `current` version carrying the supersession edges, so a
///   status-only filter returns it twice.
///
/// Measured the moment folding first ran: `brain digest` reported 2,101 memories where `brain lint`
/// reported 2,090 — 2,097 originals, plus 4 keepers counted twice, minus nothing for the 7 retired
/// claims that should have left. Neither number was arithmetic; one query was right and eleven were
/// wrong in a way no test could have caught before a fold existed to catch it.
///
/// So it lives here, once, rather than as twelve copies of subtle SQL that drift apart.
pub(crate) const CURRENT_CLAIM: &str = "v.status = 'current' AND v.version_number = (SELECT MAX(w.version_number) FROM memory_versions w WHERE w.memory_id = v.memory_id)";

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
mod push;
mod rerank;
mod search;
mod segment;
mod subjects;
mod synthesis_store;
mod tombstone;
mod upgrade;
mod vector;

pub use access::{
    MemoryAccess, MemoryRetention, RETENTION_HALF_LIFE_DAYS, STALE_RETENTION, retention_score,
};
pub use backup::{
    ABANDONED_STAGING_AGE, BACKUP_FORMAT_VERSION, BackupInventory, BackupManager, BackupReport,
    InventoryFile, InventoryKind, RecoveryDrillReport, RestoreReport, RetentionPolicy,
    RetentionReport, VerificationReport,
};
pub use basic_memory::{
    BASIC_MEMORY_PINNED_VERSION, BasicMemoryCli, BasicMemoryIndexer, BasicMemoryReport,
    BasicMemoryState, ProcessBasicMemoryCli,
};
pub use blob::{BlobRecord, BlobStore};
pub use catalog::{CatalogEvent, SegmentCatalog};
pub use context_metrics::{ContextDelivery, ContextDeliverySummary, HarnessDeliveries};
pub use embedding::{
    EMBEDDING_DIMENSIONS, Embedder, MAX_INPUT_TOKENS, cosine_similarity, decode_vector,
    default_model_dir, encode_vector, shared_embedder,
};
pub use evict::{EvictionCandidate, EvictionGate, EvictionPlan, MINIMUM_OBSERVATION, QUIET_FOR};
pub use jobs::{
    ConsolidationJob, ConsolidationQueue, ConsolidationReason, JobStatus, MAX_JOB_EVENTS,
    MAX_JOB_PAYLOAD_BYTES, RedactionManifestEntry,
};
pub use ledger::{AppendResult, EventLedger, NativeUsageEvent, StoredEvent};
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
