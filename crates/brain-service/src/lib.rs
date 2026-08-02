#![forbid(unsafe_code)]

mod capture;
mod config;
mod consolidation;
mod health;
mod hook_handler;
mod note_watcher;
mod pipe;
mod query_api;
mod reconcile;
mod runtime;

pub use brain_context::{
    ConsolidationLlm, EvidencePacket, ProposedMemory, ProposedMemoryBatch, RedactedEvidence,
};
pub use capture::{CaptureBinding, CaptureSupervisor};
pub use config::{
    CaptureServiceConfig, ConsolidationProviderConfig, ServiceLaunchConfig, ServiceProjectConfig,
};
pub use consolidation::run_configured_consolidation;
pub use consolidation::{ConsolidationCrashPoint, ConsolidationWorker, WorkerOutcome};
pub use health::{ProjectHealth, ServiceHealth, SourceHealth, source_health_key};
pub use hook_handler::{ClaudeHookHandler, HookProjectBinding, ProjectHookHandler};
pub use note_watcher::{
    GlobalPreferenceNoteWatcher, NoteScanReport, NoteWatcher, run_notes_and_projections,
};
pub use pipe::HookPipeServer;
pub use query_api::{
    BrainCheckpointRequest, BrainCheckpointResponse, BrainCitation, BrainClaimRequest,
    BrainClaimResponse, BrainClaimsRequest, BrainClaimsResponse, BrainCorrectionRequest,
    BrainCorrectionResponse, BrainEvidenceRequest, BrainEvidenceResponse, BrainItemsResponse,
    BrainQueryService, BrainReleaseClaimRequest, BrainResultItem, BrainSearchRequest,
    BrainStatusRequest, BrainStatusResponse, BrainTimelineRequest, SourceSelector, TimelineWindow,
};
pub use runtime::{build_capture_bindings, build_hook_bindings};
