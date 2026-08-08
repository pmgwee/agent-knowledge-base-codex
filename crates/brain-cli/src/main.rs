use std::path::PathBuf;

use anyhow::{Context, Result};
use brain_cli::{
    AgentSourceOptions, BenchmarkProfile, RegisterOptions, ServiceInstallOptions, TaskCommands,
    benchmark_corpus, configure_codegraph, configure_llm_wiki, disable_provider, index_codegraph,
    install_claude_hooks, install_codex_hooks, install_windows_service, provider_status,
    read_dashboard, read_diagnostics, read_hermes_status, read_status, rebuild_basic_memory,
    rebuild_markdown, register_project_with_sources, remove_provider, source_fingerprint,
    start_windows_service, stop_windows_service, uninstall_claude_hooks, uninstall_codex_hooks,
    uninstall_windows_service, verify_projections, windows_service_status,
};
use brain_coordination::{ClaimKind, PathClaimInput, SessionIdentity};
use brain_domain::{BrainConfig, Harness, ProjectId, ProjectRegistry};
use brain_service::{
    BrainCheckpointRequest, BrainClaimRequest, BrainClaimsRequest, BrainLeaseAcquireRequest,
    BrainLeaseGenerationRequest, BrainLeaseHandoffRequest, BrainLeasesRequest,
    BrainPreflightRequest, BrainQueryService, BrainReleaseClaimRequest, BrainSearchRequest,
    BrainTimelineRequest, ServiceLaunchConfig, SourceSelector, TimelineWindow,
};
use brain_store::{BackupManager, EventLedger, RetentionPolicy, UpgradeManager};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "brain", about = "Cross-agent secondary brain operator CLI")]
struct Cli {
    #[arg(long, global = true)]
    brain_home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Register {
        path: PathBuf,
        #[arg(long)]
        claude_projects_root: Option<PathBuf>,
        #[arg(long)]
        no_discover_claude: bool,
        #[arg(long)]
        codex_sessions_root: Option<PathBuf>,
        #[arg(long)]
        no_discover_codex: bool,
        #[arg(long)]
        hermes_db: Option<PathBuf>,
        #[arg(long)]
        no_hermes: bool,
    },
    Status {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        harness: Option<StatusHarness>,
        #[arg(long)]
        hermes_db: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    InstallHooks {
        harness: HookHarness,
        #[arg(long)]
        settings: Option<PathBuf>,
        #[arg(long)]
        hook_executable: Option<PathBuf>,
    },
    UninstallHooks {
        harness: HookHarness,
        #[arg(long)]
        settings: Option<PathBuf>,
        #[arg(long)]
        hook_executable: Option<PathBuf>,
    },
    Query {
        #[arg(long)]
        project: String,
        text: String,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        /// Re-rank the head of the results with the cross-encoder. Needs the checkpoint under
        /// `models/ms-marco-MiniLM-L6-v2`, and costs roughly 1.5 s.
        #[arg(long)]
        rerank: bool,
    },
    /// File a conclusion back into the brain, citing the events it rests on.
    Remember {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        /// One or more `event:<uuid>` this conclusion rests on. At least one is required.
        #[arg(long = "evidence", required = true)]
        evidence: Vec<String>,
        /// Memory ids this replaces. Their current versions are superseded, never deleted.
        #[arg(long = "supersedes")]
        supersedes: Vec<String>,
        #[arg(long, default_value = "fact")]
        kind: String,
    },
    /// Health-check a project's memories: contradictions, unrefreshed claims, islands.
    Lint {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Propose a resolution for each contradiction `brain lint` finds — derived from authority,
    /// recency and evidence weight, never from a model. Reports rather than applies.
    Reconcile {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Show which memories eviction would retire — and refuse to retire them until access has
    /// been counted long enough for "never retrieved" to mean anything.
    Evict {
        #[arg(long)]
        project: String,
        /// Actually retire them. Without this the plan is printed and nothing changes.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        json: bool,
    },
    /// Withdraw a memory. The claim stops being returned, projected or read; the ledger keeps
    /// the evidence and records the withdrawal.
    Forget {
        #[arg(long)]
        project: String,
        /// The memory id, with or without a `memory:` prefix.
        #[arg(long)]
        id: String,
        /// Why. Required — a withdrawal with no reason is indistinguishable from corruption later.
        #[arg(long)]
        reason: String,
    },
    /// Write a project's memories and their evidence to a directory.
    Export {
        #[arg(long)]
        project: String,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long, value_enum, default_value_t = brain_cli::ExportFormat::Both)]
        format: brain_cli::ExportFormat,
        /// Only memories valid from this RFC 3339 timestamp onwards.
        #[arg(long)]
        since: Option<String>,
    },
    Timeline {
        #[arg(long)]
        project: String,
        #[arg(long, value_enum, default_value_t = CliTimelineWindow::Week)]
        window: CliTimelineWindow,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        #[arg(long)]
        now: Option<String>,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Checkpoint {
        #[arg(long)]
        project: String,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        path: Vec<String>,
        #[arg(long)]
        as_of: Option<String>,
        #[arg(long)]
        max_tokens: Option<usize>,
    },
    Task {
        #[command(subcommand)]
        action: TaskCommand,
    },
    Preflight {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: Option<uuid::Uuid>,
        #[arg(long, default_value = "HEAD")]
        source: String,
        #[arg(long)]
        target: String,
    },
    Diagnose {
        #[arg(long)]
        project: Option<String>,
    },
    Dashboard,
    /// Print a content hash of the build inputs.
    ///
    /// The deploy script records this so the dashboard can tell whether rebuilding would
    /// produce different binaries. Computing it here rather than reimplementing the walk in
    /// PowerShell keeps the two sides from ever disagreeing about what "unchanged" means.
    SourceFingerprint {
        #[arg(long)]
        source_root: String,
    },
    Rebuild {
        #[command(subcommand)]
        target: RebuildCommand,
    },
    Verify {
        #[command(subcommand)]
        target: VerifyCommand,
    },
    Backup {
        #[command(subcommand)]
        action: BackupCommand,
    },
    Restore {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        destination: PathBuf,
    },
    Upgrade {
        #[command(subcommand)]
        action: UpgradeCommand,
    },
    Benchmark {
        #[arg(long, value_enum, default_value_t = CliBenchmarkProfile::Smoke)]
        profile: CliBenchmarkProfile,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    Providers {
        #[command(subcommand)]
        action: ProviderCommand,
    },
    Service {
        #[command(subcommand)]
        action: ServiceCommand,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    Install(Box<ServiceInstallCommand>),
    Start,
    Stop,
    Status,
    Uninstall,
}

#[derive(Args)]
struct ServiceInstallCommand {
    #[arg(long)]
    service_executable: Option<PathBuf>,
    #[arg(long)]
    brain_executable: Option<PathBuf>,
    #[arg(long)]
    backup_root: Option<PathBuf>,
    #[arg(long)]
    drill_root: Option<PathBuf>,
    #[arg(long)]
    install_hooks: bool,
    #[arg(long)]
    hook_executable: Option<PathBuf>,
    #[arg(long)]
    claude_settings: Option<PathBuf>,
    #[arg(long)]
    codex_settings: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ProviderCommand {
    Status {
        #[arg(long)]
        project: String,
    },
    ConfigureLlmWiki {
        #[arg(long)]
        project: String,
        #[arg(long)]
        vault: PathBuf,
    },
    ConfigureCodegraph {
        #[arg(long)]
        project: String,
        #[arg(long)]
        executable: PathBuf,
        #[arg(long)]
        activation_report: PathBuf,
    },
    IndexCodegraph {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: Option<uuid::Uuid>,
    },
    Disable {
        #[arg(long)]
        project: String,
        #[arg(value_enum)]
        provider: CliProviderKind,
    },
    Remove {
        #[arg(long)]
        project: String,
        #[arg(value_enum)]
        provider: CliProviderKind,
    },
}

#[derive(Subcommand)]
enum BackupCommand {
    Create {
        #[arg(long)]
        destination: PathBuf,
    },
    Verify {
        #[arg(long)]
        backup: PathBuf,
    },
    Maintain {
        #[arg(long)]
        root: PathBuf,
    },
    Prune {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        apply: bool,
    },
    Drill {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        work_root: PathBuf,
    },
    DrillLatest {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        work_root: PathBuf,
    },
}

#[derive(Subcommand)]
enum UpgradeCommand {
    Check,
    Stage {
        #[arg(long)]
        destination: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliBenchmarkProfile {
    Smoke,
    Primary,
    Stress,
}

#[derive(Subcommand)]
enum RebuildCommand {
    Markdown {
        #[arg(long)]
        project: String,
    },
    BasicMemory {
        #[arg(long)]
        project: String,
    },
}

#[derive(Subcommand)]
enum VerifyCommand {
    Projections {
        #[arg(long)]
        project: String,
    },
    /// Walk a memory back to the events it cites.
    Memory {
        #[arg(long)]
        project: String,
        /// The memory id, with or without a `memory:` prefix.
        #[arg(long)]
        id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    Create {
        #[arg(long)]
        project: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        worktree_parent: Option<PathBuf>,
    },
    List {
        #[arg(long)]
        project: String,
        #[arg(long)]
        include_closed: bool,
    },
    Close {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
    },
    Claim {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        kind: CliClaimKind,
        #[arg(long)]
        value: String,
        #[arg(long)]
        symbol: Option<String>,
    },
    Claims {
        #[arg(long)]
        project: String,
    },
    ReleaseClaim {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long)]
        claim: uuid::Uuid,
    },
    Acquire {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
    },
    Renew {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
        #[arg(long)]
        generation: u64,
    },
    Release {
        #[arg(long)]
        project: String,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        harness: CliHarness,
        #[arg(long)]
        session: String,
        #[arg(long)]
        generation: u64,
    },
    Handoff {
        #[arg(long)]
        project: String,
        #[arg(long)]
        handoff_id: uuid::Uuid,
        #[arg(long)]
        task: uuid::Uuid,
        #[arg(long, value_enum)]
        from_harness: CliHarness,
        #[arg(long)]
        from_session: String,
        #[arg(long)]
        generation: u64,
        #[arg(long, value_enum)]
        to_harness: CliHarness,
        #[arg(long)]
        to_session: String,
        #[arg(long)]
        checkpoint: String,
    },
    Leases {
        #[arg(long)]
        project: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum HookHarness {
    Claude,
    Codex,
}

#[derive(Clone, Copy, ValueEnum)]
enum StatusHarness {
    Hermes,
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum CliTimelineWindow {
    Day,
    #[default]
    Week,
    Month,
    Custom,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliClaimKind {
    File,
    Directory,
    Glob,
    Symbol,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliHarness {
    Claude,
    Codex,
    Hermes,
}

#[derive(Clone, Copy, ValueEnum)]
enum CliProviderKind {
    Codegraph,
    LlmWiki,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let brain_home = match cli.brain_home {
        Some(path) => path,
        None => BrainConfig::brain_home().context("resolve BRAIN_HOME")?,
    };
    match cli.command {
        Command::Register {
            path,
            claude_projects_root,
            no_discover_claude,
            codex_sessions_root,
            no_discover_codex,
            hermes_db,
            no_hermes,
        } => {
            let discovery_root = if no_discover_claude {
                None
            } else {
                claude_projects_root.or_else(default_claude_projects_root)
            };
            let codex_root = if no_discover_codex {
                None
            } else {
                codex_sessions_root.or_else(default_codex_sessions_root)
            };
            let hermes_database = if no_hermes {
                None
            } else {
                hermes_db.or_else(default_existing_hermes_database)
            };
            let result = register_project_with_sources(
                RegisterOptions {
                    brain_home,
                    project_path: path,
                    claude_projects_root: discovery_root,
                    explicit_claude_sources: Vec::new(),
                    pipe_name: None,
                },
                AgentSourceOptions {
                    configure_codex: true,
                    codex_sessions_root: codex_root,
                    explicit_codex_sources: Vec::new(),
                    configure_hermes: true,
                    hermes_database,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Status {
            project,
            harness: Some(StatusHarness::Hermes),
            hermes_db,
            json,
        } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let status = read_hermes_status(
                &brain_home,
                hermes_db.unwrap_or(default_hermes_database()?),
                project,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("harness: hermes");
                println!("activation: {}", status.activation);
                println!("project: {}", status.project_id.0);
                println!("database: {}", status.database_path.display());
                println!("observed_fingerprint: {}", status.observed_fingerprint);
                if let Some(reason) = status.reason {
                    println!("reason: {reason}");
                }
            }
        }
        Command::Status {
            project,
            harness: None,
            hermes_db: _,
            json,
        } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let status = read_status(&brain_home, project)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("project: {}", status.project_id.0);
                println!("root: {}", status.project_root.display());
                println!("events: {}", status.persisted_events);
                println!("sources: {}", status.source_count);
                println!("backlog_bytes: {}", status.backlog_bytes);
                println!("active_schema_drifts: {}", status.active_schema_drifts);
                println!("healthy: {}", status.healthy);
            }
        }
        Command::InstallHooks {
            harness: HookHarness::Claude,
            settings,
            hook_executable,
        } => {
            let result = install_claude_hooks(
                settings.unwrap_or(default_claude_settings()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::InstallHooks {
            harness: HookHarness::Codex,
            settings,
            hook_executable,
        } => {
            let result = install_codex_hooks(
                settings.unwrap_or(default_codex_hooks()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::UninstallHooks {
            harness: HookHarness::Claude,
            settings,
            hook_executable,
        } => {
            let result = uninstall_claude_hooks(
                settings.unwrap_or(default_claude_settings()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::UninstallHooks {
            harness: HookHarness::Codex,
            settings,
            hook_executable,
        } => {
            let result = uninstall_codex_hooks(
                settings.unwrap_or(default_codex_hooks()?),
                hook_executable.unwrap_or(default_hook_executable()?),
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Query {
            project,
            text,
            as_of,
            limit,
            rerank,
        } => {
            let response = BrainQueryService::open(&brain_home)?.search(BrainSearchRequest {
                project,
                text,
                as_of,
                worktree_id: None,
                task_id: None,
                native_session_id: None,
                paths: Vec::new(),
                source: SourceSelector::All,
                limit,
                rerank,
            })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Timeline {
            project,
            window,
            start,
            end,
            now,
            as_of,
            limit,
        } => {
            let response =
                BrainQueryService::open(&brain_home)?.timeline(BrainTimelineRequest {
                    project,
                    window: window.into(),
                    start,
                    end,
                    now,
                    as_of,
                    source: SourceSelector::All,
                    limit,
                })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Checkpoint {
            project,
            prompt,
            path,
            as_of,
            max_tokens,
        } => {
            let response =
                BrainQueryService::open(&brain_home)?.checkpoint(BrainCheckpointRequest {
                    project,
                    prompt,
                    paths: path,
                    as_of,
                    max_tokens,
                    harness: None,
                    native_session_id: None,
                })?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Command::Task {
            action:
                TaskCommand::Create {
                    project,
                    title,
                    base,
                    worktree_parent,
                },
        } => {
            let result = TaskCommands::open(&brain_home, &project, worktree_parent)?
                .create(&title, base.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Acquire {
                    project,
                    task,
                    harness,
                    session,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.acquire_lease(BrainLeaseAcquireRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Renew {
                    project,
                    task,
                    harness,
                    session,
                    generation,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.renew_lease(BrainLeaseGenerationRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                    generation,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Release {
                    project,
                    task,
                    harness,
                    session,
                    generation,
                },
        } => {
            let result = BrainQueryService::open(&brain_home)?.release_lease(
                BrainLeaseGenerationRequest {
                    project,
                    task_id: task,
                    owner: owner(harness, session),
                    generation,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Handoff {
                    project,
                    handoff_id,
                    task,
                    from_harness,
                    from_session,
                    generation,
                    to_harness,
                    to_session,
                    checkpoint,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.handoff_lease(BrainLeaseHandoffRequest {
                    project,
                    handoff_id,
                    task_id: task,
                    current_owner: owner(from_harness, from_session),
                    generation,
                    next_owner: owner(to_harness, to_session),
                    checkpoint,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Leases { project },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.leases(BrainLeasesRequest { project })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Preflight {
            project,
            task,
            source,
            target,
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.preflight(BrainPreflightRequest {
                    project,
                    task_id: task,
                    source_ref: source,
                    target_ref: target,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::List {
                    project,
                    include_closed,
                },
        } => {
            let result = TaskCommands::open(&brain_home, &project, None)?.list(include_closed)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Close { project, task },
        } => {
            let result = TaskCommands::open(&brain_home, &project, None)?.close(task)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::Claim {
                    project,
                    task,
                    kind,
                    value,
                    symbol,
                },
        } => {
            let result = BrainQueryService::open(&brain_home)?.claim(BrainClaimRequest {
                project,
                task_id: task,
                claims: vec![PathClaimInput {
                    kind: kind.into(),
                    value,
                    symbol,
                }],
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action: TaskCommand::Claims { project },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.claims(BrainClaimsRequest { project })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Task {
            action:
                TaskCommand::ReleaseClaim {
                    project,
                    task,
                    claim,
                },
        } => {
            let result =
                BrainQueryService::open(&brain_home)?.release_claim(BrainReleaseClaimRequest {
                    project,
                    task_id: task,
                    claim_id: claim,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Diagnose { project } => {
            let project = project.as_deref().map(parse_project_id).transpose()?;
            let bundle = read_diagnostics(&brain_home, project)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
        }
        Command::Dashboard => {
            let snapshot = read_dashboard(&brain_home)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        Command::SourceFingerprint { source_root } => {
            let root = std::path::Path::new(&source_root);
            let fingerprint = source_fingerprint(root).ok_or_else(|| {
                anyhow::anyhow!("could not read build inputs under {}", root.display())
            })?;
            println!("{fingerprint}");
        }
        Command::Rebuild {
            target: RebuildCommand::Markdown { project },
        } => {
            let report = rebuild_markdown(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Rebuild {
            target: RebuildCommand::BasicMemory { project },
        } => {
            let report = rebuild_basic_memory(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Remember {
            project,
            title,
            content,
            evidence,
            supersedes,
            kind,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let parse_ids = |values: &[String], label: &str| -> Result<Vec<uuid::Uuid>> {
                values
                    .iter()
                    .map(|value| {
                        uuid::Uuid::parse_str(
                            value
                                .trim()
                                .trim_start_matches("event:")
                                .trim_start_matches("memory:"),
                        )
                        .with_context(|| format!("{label} must be a UUID, got {value:?}"))
                    })
                    .collect()
            };
            let request = brain_cli::RememberRequest {
                project_id,
                worktree_id: project_config.worktree_id,
                kind: brain_domain::MemoryKind::from_name(&kind)
                    .with_context(|| format!("unknown memory kind {kind:?}"))?,
                title: &title,
                content: &content,
                evidence_ids: parse_ids(&evidence, "--evidence")?,
                supersedes: parse_ids(&supersedes, "--supersedes")?,
                now: time::OffsetDateTime::now_utc(),
            };
            let filed = brain_cli::remember(&mut ledger, request)?;
            println!("{}", serde_json::to_string_pretty(&filed)?);
        }
        Command::Reconcile { project, json } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report = brain_cli::propose_reconciliation(&ledger, project_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_reconcile(&report));
            }
        }
        Command::Evict {
            project,
            apply,
            json,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let now = time::OffsetDateTime::now_utc();
            let plan = ledger.plan_eviction(now)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print!("{}", brain_cli::render_eviction(&plan));
            }
            if apply {
                // `apply_eviction` re-plans and enforces the gate itself, so this cannot retire
                // anything the printed plan did not cover, and cannot run early.
                let retired = ledger.apply_eviction(now, "brain evict")?;
                println!(
                    "
retired {retired} memories as tombstones"
                );
            }
        }
        Command::Lint { project, json } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report =
                brain_cli::lint_project(&ledger, project_id, time::OffsetDateTime::now_utc())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_lint(&report));
            }
            // Findings that need a decision exit non-zero so this is usable in a check, while
            // observations — unrefreshed claims, islands, a running backfill — do not.
            if report.actionable {
                anyhow::bail!("lint found something that needs a decision");
            }
        }
        Command::Forget {
            project,
            id,
            reason,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let memory_id = uuid::Uuid::parse_str(id.trim().trim_start_matches("memory:"))
                .context("memory id must be a UUID, optionally prefixed with `memory:`")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let mut ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let who = std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "unknown".to_owned());
            let tombstone =
                ledger.forget_memory(memory_id, &reason, &who, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&tombstone)?);
            eprintln!(
                "Withdrawn. The memory will stop appearing in search, orientation and the vault 
                 at the next projection. Its evidence is untouched and the withdrawal is on record."
            );
        }
        Command::Export {
            project,
            destination,
            format,
            since,
        } => {
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let since = since
                .as_deref()
                .map(|value| {
                    time::OffsetDateTime::parse(
                        value,
                        &time::format_description::well_known::Rfc3339,
                    )
                })
                .transpose()
                .context("--since must be an RFC 3339 timestamp")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report =
                brain_cli::export_project(&ledger, project_id, &destination, format, since)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            // Evidence that will not resolve is the one thing an export must never hide, since
            // after it leaves here nothing can resolve it.
            if report.memories_missing_evidence > 0 {
                anyhow::bail!(
                    "{} memory(ies) cite evidence this ledger could not resolve",
                    report.memories_missing_evidence
                );
            }
        }
        Command::Verify {
            target: VerifyCommand::Memory { project, id, json },
        } => {
            // Accepts the same selector `brain query` does — a project path or an id — because
            // the id you have to hand when checking a claim is the memory's, not the project's.
            let project_id = ProjectRegistry::open(&brain_home)?.resolve(&project)?;
            let memory_id = uuid::Uuid::parse_str(id.trim().trim_start_matches("memory:"))
                .context("memory id must be a UUID, optionally prefixed with `memory:`")?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            let project_config = config.project(Some(project_id))?;
            let ledger = EventLedger::open(&project_config.ledger_path, project_id)?;
            let report = brain_cli::verify_memory(&ledger, project_id, memory_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", brain_cli::render_provenance(&report));
            }
            // A citation that does not resolve is corruption of the one property this brain
            // sells, so it fails the command rather than being a line in the output.
            if !report.intact() {
                anyhow::bail!(
                    "{} citation(s) do not resolve to events in this ledger",
                    report.unresolved.len()
                );
            }
        }
        Command::Verify {
            target: VerifyCommand::Projections { project },
        } => {
            let report = verify_projections(&brain_home, parse_project_id(&project)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.valid {
                anyhow::bail!("projection verification failed");
            }
        }
        Command::Backup {
            action: BackupCommand::Create { destination },
        } => {
            let report =
                BackupManager::create(&brain_home, destination, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Verify { backup },
        } => {
            let report = BackupManager::verify(backup)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Maintain { root },
        } => {
            let backup =
                BackupManager::create(&brain_home, &root, time::OffsetDateTime::now_utc())?;
            let retention =
                BackupManager::apply_retention(&root, RetentionPolicy::default(), false)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "backup": backup,
                    "retention": retention,
                }))?
            );
        }
        Command::Backup {
            action: BackupCommand::Prune { root, apply },
        } => {
            let report = BackupManager::apply_retention(root, RetentionPolicy::default(), !apply)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Backup {
            action: BackupCommand::Drill { backup, work_root },
        } => {
            let report =
                BackupManager::recovery_drill(backup, work_root, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.success {
                anyhow::bail!(
                    "recovery drill failed: {}",
                    report.error.as_deref().unwrap_or("unknown error")
                );
            }
        }
        Command::Backup {
            action: BackupCommand::DrillLatest { root, work_root },
        } => {
            let backup = BackupManager::latest_verified_backup(root)?;
            let report =
                BackupManager::recovery_drill(backup, work_root, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.success {
                anyhow::bail!(
                    "recovery drill failed: {}",
                    report.error.as_deref().unwrap_or("unknown error")
                );
            }
        }
        Command::Restore {
            backup,
            destination,
        } => {
            let report = BackupManager::restore_isolated(backup, destination)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Upgrade {
            action: UpgradeCommand::Check,
        } => {
            let report = UpgradeManager::check(&brain_home)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.compatible {
                anyhow::bail!("brain formats are not compatible with this binary");
            }
        }
        Command::Upgrade {
            action: UpgradeCommand::Stage { destination },
        } => {
            let report =
                UpgradeManager::stage(&brain_home, destination, time::OffsetDateTime::now_utc())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Benchmark {
            profile,
            output,
            seed,
        } => {
            let report = benchmark_corpus(output, profile.into(), seed)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.passed {
                anyhow::bail!("benchmark gates failed: {}", report.failures.join("; "));
            }
        }
        Command::Providers {
            action: ProviderCommand::Status { project },
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&provider_status(&brain_home, &project)?)?
            );
        }
        Command::Providers {
            action: ProviderCommand::ConfigureLlmWiki { project, vault },
        } => {
            let report = configure_llm_wiki(&brain_home, &project, &vault)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action:
                ProviderCommand::ConfigureCodegraph {
                    project,
                    executable,
                    activation_report,
                },
        } => {
            let report =
                configure_codegraph(&brain_home, &project, &executable, &activation_report)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::IndexCodegraph { project, task },
        } => {
            let report = index_codegraph(&brain_home, &project, task)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::Disable { project, provider },
        } => {
            let report = disable_provider(&brain_home, &project, provider.into())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Providers {
            action: ProviderCommand::Remove { project, provider },
        } => {
            let report = remove_provider(&brain_home, &project, provider.into())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Service {
            action: ServiceCommand::Install(arguments),
        } => {
            let ServiceInstallCommand {
                service_executable,
                brain_executable,
                backup_root,
                drill_root,
                install_hooks,
                hook_executable,
                claude_settings,
                codex_settings,
            } = *arguments;
            let current_executable = std::env::current_exe()?;
            let binary_directory = current_executable
                .parent()
                .context("brain executable has no parent")?;
            let backup_root =
                backup_root.unwrap_or_else(|| sibling_path(&brain_home, "AgentBrainBackups"));
            let drill_root =
                drill_root.unwrap_or_else(|| sibling_path(&brain_home, "AgentBrainDrills"));
            let hook_executable = if install_hooks {
                Some(hook_executable.unwrap_or_else(|| binary_directory.join("brain-hook.exe")))
            } else {
                None
            };
            let claude_settings = if install_hooks {
                Some(match claude_settings {
                    Some(path) => path,
                    None => default_claude_settings()?,
                })
            } else {
                None
            };
            let codex_settings = if install_hooks {
                Some(match codex_settings {
                    Some(path) => path,
                    None => default_codex_hooks()?,
                })
            } else {
                None
            };
            let options = ServiceInstallOptions {
                brain_home: brain_home.clone(),
                service_executable: service_executable
                    .unwrap_or_else(|| binary_directory.join("brain-service.exe")),
                brain_executable: brain_executable.unwrap_or(current_executable),
                backup_root,
                drill_root,
                hook_executable,
                claude_settings,
                codex_settings,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&install_windows_service(options)?)?
            );
        }
        Command::Service {
            action: ServiceCommand::Start,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&start_windows_service(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Stop,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&stop_windows_service(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Status,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&windows_service_status(&brain_home)?)?
        ),
        Command::Service {
            action: ServiceCommand::Uninstall,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&uninstall_windows_service(&brain_home)?)?
        ),
    }
    Ok(())
}

impl From<CliProviderKind> for brain_cli::ProviderKind {
    fn from(value: CliProviderKind) -> Self {
        match value {
            CliProviderKind::Codegraph => Self::Codegraph,
            CliProviderKind::LlmWiki => Self::LlmWiki,
        }
    }
}

impl From<CliTimelineWindow> for TimelineWindow {
    fn from(value: CliTimelineWindow) -> Self {
        match value {
            CliTimelineWindow::Day => Self::Day,
            CliTimelineWindow::Week => Self::Week,
            CliTimelineWindow::Month => Self::Month,
            CliTimelineWindow::Custom => Self::Custom,
        }
    }
}

impl From<CliClaimKind> for ClaimKind {
    fn from(value: CliClaimKind) -> Self {
        match value {
            CliClaimKind::File => Self::File,
            CliClaimKind::Directory => Self::Directory,
            CliClaimKind::Glob => Self::Glob,
            CliClaimKind::Symbol => Self::Symbol,
        }
    }
}

impl From<CliBenchmarkProfile> for BenchmarkProfile {
    fn from(value: CliBenchmarkProfile) -> Self {
        match value {
            CliBenchmarkProfile::Smoke => Self::Smoke,
            CliBenchmarkProfile::Primary => Self::Primary,
            CliBenchmarkProfile::Stress => Self::Stress,
        }
    }
}

fn owner(harness: CliHarness, native_session_id: String) -> SessionIdentity {
    SessionIdentity {
        harness: match harness {
            CliHarness::Claude => Harness::ClaudeCode,
            CliHarness::Codex => Harness::Codex,
            CliHarness::Hermes => Harness::Hermes,
        },
        native_session_id,
    }
}

fn parse_project_id(value: &str) -> Result<ProjectId> {
    Ok(ProjectId(
        uuid::Uuid::parse_str(value).with_context(|| format!("invalid project ID {value}"))?,
    ))
}

fn default_claude_projects_root() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".claude").join("projects"))
        .filter(|path| path.is_dir())
}

fn sibling_path(brain_home: &std::path::Path, name: &str) -> PathBuf {
    brain_home
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(name)
}

fn default_codex_sessions_root() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("sessions"))
        .filter(|path| path.is_dir())
}

fn default_existing_hermes_database() -> Option<PathBuf> {
    default_hermes_database().ok().filter(|path| path.is_file())
}

fn default_claude_settings() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is unavailable")?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

fn default_codex_hooks() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").context("USERPROFILE is unavailable")?;
    Ok(PathBuf::from(home).join(".codex").join("hooks.json"))
}

fn default_hermes_database() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
    Ok(PathBuf::from(local).join("hermes").join("state.db"))
}

fn default_hook_executable() -> Result<PathBuf> {
    let current = std::env::current_exe().context("resolve brain executable")?;
    let name = if cfg!(windows) {
        "brain-hook.exe"
    } else {
        "brain-hook"
    };
    Ok(current.with_file_name(name))
}
