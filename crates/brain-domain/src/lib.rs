#![forbid(unsafe_code)]

mod config;
mod event;
mod hook;
mod ids;
mod project;
mod registry;

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
pub use project::ProjectIdentity;
pub use registry::ProjectRegistry;
