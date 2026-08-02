#![forbid(unsafe_code)]

mod claude;
mod jsonl;
mod traits;

pub use claude::ClaudeAdapter;
pub use traits::{
    FileIdentity, FileRotation, NormalizeContext, RawRecord, RawRecordBatch, ReadOutcome,
    SchemaDrift, SchemaFingerprint, SourceAdapter, SourceDescriptor, SourceUnavailable,
};
