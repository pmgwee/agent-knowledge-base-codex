#![forbid(unsafe_code)]

mod claude;
mod conformance;
mod jsonl;
mod traits;

pub use claude::ClaudeAdapter;
pub use conformance::{
    AdapterConformanceReport, AdapterConformanceSubject, assert_adapter_conformance,
};
pub use traits::{
    FileIdentity, FileRotation, NormalizeContext, RawRecord, RawRecordBatch, ReadOutcome,
    SchemaDrift, SchemaFingerprint, SourceAdapter, SourceDescriptor, SourceUnavailable,
};
