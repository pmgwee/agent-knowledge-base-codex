use std::sync::Arc;

use brain_domain::BrainConfig;
use brain_service::{
    CaptureSupervisor, HookPipeServer, ProjectHookHandler, ServiceLaunchConfig,
    build_capture_bindings, build_hook_bindings,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::EnvFilter;

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .json()
        .try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let brain_home = BrainConfig::brain_home()?;
    let config = ServiceLaunchConfig::load(ServiceLaunchConfig::default_path(&brain_home))?;
    let capture = Arc::new(CaptureSupervisor::new(build_capture_bindings(&config)?)?);
    let handler = Arc::new(ProjectHookHandler::for_projects(build_hook_bindings(
        &config,
    ))?);
    let pipe = HookPipeServer::new(config.pipe_name);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let signal_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = signal_tx.send(true);
        }
    });
    let pipe_shutdown = shutdown_rx.clone();
    let pipe_handler = Arc::clone(&handler);
    tokio::try_join!(
        pipe.run(pipe_shutdown, move |envelope| {
            let handler = Arc::clone(&pipe_handler);
            async move { handler.handle(&envelope) }
        }),
        Arc::clone(&capture).run(shutdown_rx)
    )?;
    Ok(())
}
