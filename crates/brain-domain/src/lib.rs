#![forbid(unsafe_code)]

mod config;
mod event;
mod hook;
mod ids;
mod memory;
mod project;
mod registry;
mod version;

pub use config::BrainConfig;
pub use event::{
    CaptureGapRecord, EventBatch, EventType, Harness, NormalizedEvent, QuarantinedRecord,
    SchemaDriftRecord, SourceCursor,
};
pub use hook::{
    HOOK_MAX_FRAME_BYTES, HOOK_PROTOCOL_VERSION, HookEnvelope, HookReply, decode_hook_frame_length,
    decode_hook_frame_payload, encode_hook_frame,
};
pub use ids::{ProjectId, WorktreeId};
pub use memory::{Authority, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus};
pub use project::ProjectIdentity;
pub use registry::ProjectRegistry;
pub use version::{FormatVersions, SUPPORTED_FORMATS};
