#![forbid(unsafe_code)]

mod capture;
mod config;
mod consolidation;
mod health;
mod hook_handler;
mod pipe;
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
pub use pipe::HookPipeServer;
pub use runtime::{build_capture_bindings, build_hook_bindings};
