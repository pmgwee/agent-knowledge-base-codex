#![forbid(unsafe_code)]

mod backup;
mod basic_memory;
mod blob;
mod catalog;
mod cursor;
mod jobs;
mod ledger;
mod markdown;
mod memory;
mod migrations;
mod notes;
mod provider_cache;
mod search;
mod segment;
mod upgrade;

pub use backup::{
    BACKUP_FORMAT_VERSION, BackupInventory, BackupManager, BackupReport, InventoryFile,
    InventoryKind, RestoreReport, VerificationReport,
};
pub use basic_memory::{
    BASIC_MEMORY_PINNED_VERSION, BasicMemoryCli, BasicMemoryIndexer, BasicMemoryReport,
    BasicMemoryState, ProcessBasicMemoryCli,
};
pub use blob::{BlobRecord, BlobStore};
pub use catalog::{CatalogEvent, SegmentCatalog};
pub use jobs::{ConsolidationJob, ConsolidationReason, JobStatus, RedactionManifestEntry};
pub use ledger::{AppendResult, EventLedger, StoredEvent};
pub use markdown::{
    MarkdownProjector, ProjectionReport, ProjectionVerification, project_vault_root,
};
pub use memory::GlobalPreferenceStore;
pub use provider_cache::{ProviderCacheEntry, ProviderCacheStore};
pub use search::{SearchHit, SearchQuery, SearchSource, SearchSourceFilter, TimeRange};
pub use segment::{
    SEGMENT_FORMAT_VERSION, SegmentManifest, SegmentSealReport, SegmentStore, should_seal,
};
pub use upgrade::{UpgradeIssue, UpgradeManager, UpgradeReport, UpgradeStageReport};
