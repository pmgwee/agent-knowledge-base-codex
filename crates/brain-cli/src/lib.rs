#![forbid(unsafe_code)]

mod diagnose;
mod install_hooks;
mod rebuild;
mod register;
mod status;

pub use diagnose::{DiagnosticBundle, RedactedCursor, RedactedSchemaDrift, read_diagnostics};
pub use install_hooks::{
    HookInstallResult, install_claude_hooks, install_codex_hooks, uninstall_claude_hooks,
    uninstall_codex_hooks,
};
pub use rebuild::{
    rebuild_basic_memory, rebuild_basic_memory_with, rebuild_markdown, verify_projections,
};
pub use register::{
    AgentSourceOptions, RegisterOptions, RegistrationResult, register_project,
    register_project_with_sources,
};
pub use status::{BrainStatus, HermesStatus, read_hermes_status, read_status};
pub use task::TaskCommands;

mod task;
