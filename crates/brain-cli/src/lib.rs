#![forbid(unsafe_code)]

mod benchmark;
mod dashboard;
mod deployment;
mod diagnose;
mod install_hooks;
mod longmemeval;
mod providers;
mod rebuild;
mod register;
mod service;
mod status;

pub use dashboard::{DashboardSnapshot, read_dashboard};
pub use deployment::{
    DEPLOY_MANIFEST, DEPLOYED_BINARIES, DeployManifest, DeployStatus, DeployedBinary,
    DeploymentDashboard, read_deployment, read_head_commit, source_fingerprint,
};
pub use diagnose::{DiagnosticBundle, RedactedCursor, RedactedSchemaDrift, read_diagnostics};
pub use install_hooks::{
    HookInstallResult, install_claude_hooks, install_codex_hooks, uninstall_claude_hooks,
    uninstall_codex_hooks,
};
pub use longmemeval::{LongMemEvalInstance, LongMemEvalReport, run_longmemeval};
pub use providers::{
    CodeGraphIndexReport, ProviderChangeReport, ProviderKind, ProviderStatusReport,
    configure_codegraph, configure_llm_wiki, disable_provider, index_codegraph, provider_status,
    remove_provider,
};
pub use rebuild::{
    rebuild_basic_memory, rebuild_basic_memory_with, rebuild_markdown, verify_projections,
};
pub use register::{
    AgentSourceOptions, RegisterOptions, RegistrationResult, register_project,
    register_project_with_sources,
};
pub use service::{
    ServiceInstallOptions, ServiceInstallReport, ServiceStatusReport, ServiceTaskStatus,
    ServiceUninstallReport, install_windows_service, start_windows_service, stop_windows_service,
    uninstall_windows_service, windows_service_status,
};
pub use status::{BrainStatus, HermesStatus, read_hermes_status, read_status};
pub use task::TaskCommands;

mod task;
pub use benchmark::{
    BenchmarkProfile, BenchmarkReport, CorpusHashes, benchmark_corpus, benchmark_report_dir,
    corpus_hashes, preserve_benchmark_report,
};
