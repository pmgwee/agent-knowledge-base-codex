#![forbid(unsafe_code)]

mod benchmark;
mod config_panel;
mod dashboard;
mod deployment;
mod diagnose;
mod digest;
mod evict;
mod explain;
mod export;
mod install_hooks;
mod lint;
mod longmemeval;
mod provenance;
mod providers;
mod rebuild;
mod reconcile;
mod register;
mod remember;
mod service;
mod status;

pub use config_panel::{
    BudgetContract, ConfigDashboard, CredentialBinding, HarnessWiring, McpWiring, ScheduledJob,
    read_config_panel,
};
pub use dashboard::{DashboardSnapshot, read_dashboard};
pub use deployment::{
    DEPLOY_MANIFEST, DEPLOYED_BINARIES, DeployManifest, DeployStatus, DeployedBinary,
    DeploymentDashboard, read_deployment, read_head_commit, source_fingerprint,
};
pub use diagnose::{DiagnosticBundle, RedactedCursor, RedactedSchemaDrift, read_diagnostics};
pub use digest::{
    Digest, build as build_digest, render as render_digest,
    render_markdown as render_digest_markdown,
};
pub use evict::render as render_eviction;
pub use explain::{
    ExplainReport, ExplainedHit, explain as explain_query, render as render_explain,
};
pub use export::{ExportFormat, ExportReport, export_project};
pub use install_hooks::{
    HookInstallResult, install_claude_hooks, install_codex_hooks, uninstall_claude_hooks,
    uninstall_codex_hooks,
};
pub use lint::{
    DateRepairReport, LintFinding, LintReport, lint_project, render as render_lint,
    render_date_repair, repair_dates,
};
pub use longmemeval::{
    LongMemEvalInstance, LongMemEvalOptions, LongMemEvalReport, run_longmemeval,
};
pub use provenance::{EvidenceTrace, ProvenanceReport, render as render_provenance, verify_memory};
pub use providers::{
    CodeGraphIndexReport, ProviderChangeReport, ProviderKind, ProviderStatusReport,
    configure_codegraph, configure_llm_wiki, disable_provider, index_codegraph, provider_status,
    remove_provider,
};
pub use rebuild::{
    rebuild_basic_memory, rebuild_basic_memory_with, rebuild_markdown, verify_projections,
};
pub use reconcile::{
    ApplyReport, Proposal, ReconcileReport, ResolutionRule, apply as apply_reconciliation,
    propose as propose_reconciliation, render as render_reconcile,
    render_apply as render_reconcile_apply,
};
pub use register::{
    AgentSourceOptions, RegisterOptions, RegistrationResult, register_project,
    register_project_with_sources,
};
pub use remember::{RememberRequest, RememberedMemory, derive_evidence, remember};
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
