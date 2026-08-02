#![forbid(unsafe_code)]

mod config;
mod event;
mod ids;
mod project;
mod registry;

pub use config::BrainConfig;
pub use event::{EventBatch, EventType, Harness, NormalizedEvent, SourceCursor};
pub use ids::{ProjectId, WorktreeId};
pub use project::ProjectIdentity;
pub use registry::ProjectRegistry;
