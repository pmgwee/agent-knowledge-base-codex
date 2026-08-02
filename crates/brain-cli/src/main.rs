use std::path::PathBuf;

use anyhow::{Context, Result};
use brain_cli::{
    AgentSourceOptions, RegisterOptions, TaskCommands, install_claude_hooks, install_codex_hooks,
    read_diagnostics, read_hermes_status, read_status, rebuild_basic_memory, rebuild_markdown,
    register_project_with_sources, uninstall_claude_hooks, uninstall_codex_hooks,
    verify_projections,
};
use brain_coordination::{ClaimKind, PathClaimInput, SessionIdentity};
use brain_domain::{BrainConfig, Harness, ProjectId};
use brain_service::{
    BrainCheckpointRequest, BrainClaimRequest, BrainClaimsRequest, BrainLeaseAcquireRequest,
    BrainLeaseGenerationRequest, BrainLeaseHandoffRequest, BrainLeasesRequest,
    BrainPreflightRequest, BrainQueryService, BrainReleaseClaimRequest, BrainSearchRequest,
    BrainTimelineRequest, SourceSelector, TimelineWindow,
};
use brain_store::BackupManager;
use clap::{Parser, Subcommand, ValueEnum};

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
        Command::Restore {
            backup,
            destination,
        } => {
            let report = BackupManager::restore_isolated(backup, destination)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
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
