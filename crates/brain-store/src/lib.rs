#![forbid(unsafe_code)]

mod cursor;
mod jobs;
mod ledger;
mod memory;
mod migrations;
mod search;

pub use jobs::{ConsolidationJob, ConsolidationReason, JobStatus, RedactionManifestEntry};
pub use ledger::{AppendResult, EventLedger, StoredEvent};
pub use memory::GlobalPreferenceStore;
pub use search::{SearchHit, SearchQuery, SearchSource, SearchSourceFilter, TimeRange};
