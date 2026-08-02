#![forbid(unsafe_code)]

mod capture;
mod config;
mod consolidation;
mod health;
mod hook_handler;
mod pipe;
mod reconcile;
mod runtime;

pub use capture::{CaptureBinding, CaptureSupervisor};
pub use config::{CaptureServiceConfig, ServiceLaunchConfig, ServiceProjectConfig};
pub use consolidation::{
    ConsolidationCrashPoint, ConsolidationWorker, EvidencePacket, MemoryProposer, RedactedEvidence,
    WorkerOutcome,
};
pub use health::{ProjectHealth, ServiceHealth, SourceHealth, source_health_key};
pub use hook_handler::{ClaudeHookHandler, HookProjectBinding, ProjectHookHandler};
pub use pipe::HookPipeServer;
pub use runtime::{build_capture_bindings, build_hook_bindings};
