use std::sync::Arc;

use brain_domain::BrainConfig;
use brain_service::{
    CaptureSupervisor, HookPipeServer, ProjectHookHandler, ServiceLaunchConfig, TranscriptRoots,
    build_capture_bindings, build_hook_bindings, rediscover_once,
    run_configured_consolidation_with_pressure, run_notes_and_projections_with_pressure,
    run_rediscovery,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::EnvFilter;

    let arguments = launch_arguments()?;
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let _log_guard = if let Some(log_dir) = &arguments.log_dir {
        std::fs::create_dir_all(log_dir)?;
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("brain-service")
            .filename_suffix("jsonl")
            .max_log_files(14)
            .build(log_dir)?;
        let (writer, guard) = tracing_appender::non_blocking(appender);
        tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_writer(writer)
            .json()
            .try_init()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Some(guard)
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter())
            .json()
            .try_init()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        None
    };

    let brain_home = arguments.brain_home;
    let config_path = ServiceLaunchConfig::default_path(&brain_home);
    // Pick up sessions created since the last run before building bindings, so a restart
    // immediately captures work that arrived while the service was down.
    let transcript_roots = TranscriptRoots::from_user_profile();
    if let Err(error) = rediscover_once(&config_path, &transcript_roots) {
        tracing::warn!(%error, "initial source rediscovery failed");
    }
    let config = ServiceLaunchConfig::load(&config_path)?;
    let capture = Arc::new(CaptureSupervisor::new(build_capture_bindings(&config)?)?);
    let consolidation_pressure = capture.degradation_receiver();
    let projection_pressure = capture.degradation_receiver();
    let mut hook_bindings = build_hook_bindings(&config);
    let global_preferences_path = brain_home
        .join("global-preferences")
        .join("preferences.sqlite");
    for binding in &mut hook_bindings {
        binding.global_preferences_path = Some(global_preferences_path.clone());
    }
    let handler = Arc::new(ProjectHookHandler::for_projects(hook_bindings)?);
    let pipe = HookPipeServer::new(config.pipe_name.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let signal_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = signal_tx.send(true);
        }
    });
    let pipe_shutdown = shutdown_rx.clone();
    let consolidation_shutdown = shutdown_rx.clone();
    let projection_shutdown = shutdown_rx.clone();
    let rediscovery_shutdown = shutdown_rx.clone();
    let consolidation_config = config.clone();
    let projection_config = config.clone();
    let pipe_handler = Arc::clone(&handler);
    tokio::try_join!(
        pipe.run(pipe_shutdown, move |envelope| {
            let handler = Arc::clone(&pipe_handler);
            async move { handler.handle(&envelope) }
        }),
        Arc::clone(&capture).run(shutdown_rx),
        run_configured_consolidation_with_pressure(
            consolidation_config,
            consolidation_shutdown,
            consolidation_pressure,
        ),
        run_notes_and_projections_with_pressure(
            projection_config,
            brain_home,
            projection_shutdown,
            projection_pressure,
        ),
        run_rediscovery(config_path, transcript_roots, rediscovery_shutdown)
    )?;
    Ok(())
}

struct LaunchArguments {
    brain_home: std::path::PathBuf,
    log_dir: Option<std::path::PathBuf>,
}

fn launch_arguments() -> anyhow::Result<LaunchArguments> {
    let mut brain_home = None;
    let mut log_dir = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_string_lossy().as_ref() {
            "--brain-home" => {
                brain_home = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--brain-home requires a path"))?
                        .into(),
                );
            }
            "--log-dir" => {
                log_dir = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--log-dir requires a path"))?
                        .into(),
                );
            }
            other => anyhow::bail!("unknown brain-service argument {other:?}"),
        }
    }
    Ok(LaunchArguments {
        brain_home: brain_home.unwrap_or(BrainConfig::brain_home()?),
        log_dir,
    })
}
