#![forbid(unsafe_code)]

mod basic_memory;
mod cursor;
mod jobs;
mod ledger;
mod markdown;
mod memory;
mod migrations;
mod notes;
mod search;

pub use basic_memory::{
    BASIC_MEMORY_PINNED_VERSION, BasicMemoryCli, BasicMemoryIndexer, BasicMemoryReport,
    BasicMemoryState, ProcessBasicMemoryCli,
};
pub use jobs::{ConsolidationJob, ConsolidationReason, JobStatus, RedactionManifestEntry};
pub use ledger::{AppendResult, EventLedger, StoredEvent};
pub use markdown::{
    MarkdownProjector, ProjectionReport, ProjectionVerification, project_vault_root,
};
pub use memory::GlobalPreferenceStore;
pub use search::{SearchHit, SearchQuery, SearchSource, SearchSourceFilter, TimeRange};
