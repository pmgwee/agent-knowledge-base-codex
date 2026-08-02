use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use brain_cli::{
    RegisterOptions, install_claude_hooks, install_codex_hooks, read_hermes_status, read_status,
    register_project, uninstall_claude_hooks, uninstall_codex_hooks,
};
use brain_context::{ContextCompiler, ContextQuery};
use brain_domain::{BrainConfig, ProjectId};
use brain_service::ServiceLaunchConfig;
use brain_store::EventLedger;
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
        } => {
            let discovery_root = if no_discover_claude {
                None
            } else {
                claude_projects_root.or_else(default_claude_projects_root)
            };
            let result = register_project(RegisterOptions {
                brain_home,
                project_path: path,
                claude_projects_root: discovery_root,
                explicit_claude_sources: Vec::new(),
                pipe_name: None,
            })?;
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
            text: _text,
        } => {
            let project = parse_project_id(&project)?;
            let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
            if config.project_id != project {
                bail!(
                    "project {} is not the configured service scope {}",
                    project.0,
                    config.project_id.0
                );
            }
            let ledger = EventLedger::open(&config.ledger_path, project)?;
            let compiler = ContextCompiler::from_ledger(&ledger, project, 500)?;
            let context =
                compiler.compile(ContextQuery::for_worktree(project, config.worktree_id))?;
            println!("{}", context.text);
        }
    }
    Ok(())
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
