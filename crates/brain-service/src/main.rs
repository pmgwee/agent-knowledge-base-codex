use std::sync::Arc;

use brain_domain::BrainConfig;
use brain_service::{
    CaptureSupervisor, HookPipeServer, ProjectHookHandler, ServiceLaunchConfig, TranscriptRoots,
    build_capture_bindings, build_hook_bindings, rediscover_once,
    run_configured_consolidation_with_pressure, run_embedding_backfill,
    run_notes_and_projections_with_pressure, run_rediscovery,
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
    let embedding_shutdown = shutdown_rx.clone();
    let embedding_pressure = capture.degradation_receiver();
    let embedding_config = config.clone();
    let embedding_home = brain_home.clone();
    let consolidation_config = config.clone();
    let projection_config = config.clone();
    let pipe_handler = Arc::clone(&handler);
    // The pipe server runs on its own task. `try_join!` polls every future on a single task,
    // so when it shared that task with the capture/consolidation/projection/rediscovery loops
    // below, any blocking SQLite call in those siblings stalled the pipe's accept loop for
    // ~1 s — which is why every hook hit its 250 ms ceiling and failed open with `{}`. Giving
    // the pipe its own task means a sibling holding the reactor no longer blocks hook delivery.
    //
    // The handler itself is moved to the blocking pool via `spawn_blocking`: compiling an
    // orientation is ~130 ms of synchronous SQLite, and running it on a reactor worker would
    // let one slow session block the accept loop the decoupling just freed. The blocking pool
    // exists for exactly this.
    let pipe_task = tokio::spawn(async move {
        pipe.run(pipe_shutdown, move |envelope| {
            let handler = Arc::clone(&pipe_handler);
            async move {
                match tokio::task::spawn_blocking(move || handler.handle(&envelope)).await {
                    Ok(outcome) => outcome,
                    Err(join_error) => Err(anyhow::anyhow!(
                        "hook handler blocking task failed: {join_error}"
                    )),
                }
            }
        })
        .await
    });

    // `join!`, not `try_join!`.
    //
    // Every subsystem used to share one fate: `try_join!` returns on the first error, so a
    // transient failure anywhere ended the process. That was not theoretical — three separate
    // instances were found by the service dying silently and exiting 1, each in a different
    // loop, each a failure that mattered to one subsystem and to nothing else. A vanished
    // transcript directory stopped hook delivery. A pipe accept error stopped capture.
    //
    // Those loops are individually contained now, but containment is a property someone has to
    // keep re-establishing every time a `?` is added. This is the structural version: a
    // subsystem that returns is reported and mourned, and the others carry on. Losing
    // consolidation costs distilled memories; losing the process costs every session started
    // before anyone notices, which is the failure this brain exists to prevent.
    let (pipe, capture_result, consolidation, projections, rediscovery, embedding) = tokio::join!(
        async {
            match pipe_task.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(error),
                Err(join_error) => Err(anyhow::anyhow!(
                    "hook pipe server task panicked: {join_error}"
                )),
            }
        },
        supervise("capture", shutdown_rx.clone(), || {
            Arc::clone(&capture).run(shutdown_rx.clone())
        }),
        supervise("consolidation", consolidation_shutdown.clone(), || {
            run_configured_consolidation_with_pressure(
                consolidation_config.clone(),
                consolidation_shutdown.clone(),
                consolidation_pressure.clone(),
            )
        }),
        supervise("notes and projections", projection_shutdown.clone(), || {
            run_notes_and_projections_with_pressure(
                projection_config.clone(),
                brain_home.clone(),
                projection_shutdown.clone(),
                projection_pressure.clone(),
            )
        }),
        supervise("source rediscovery", rediscovery_shutdown.clone(), || {
            run_rediscovery(
                config_path.clone(),
                transcript_roots.clone(),
                rediscovery_shutdown.clone(),
            )
        }),
        supervise("memory embedding", embedding_shutdown.clone(), || {
            run_embedding_backfill(
                embedding_config.clone(),
                embedding_home.clone(),
                embedding_shutdown.clone(),
                embedding_pressure.clone(),
            )
        })
    );

    // Every subsystem has now returned, which only happens on shutdown or on failure. Report
    // each one by name: "the service stopped" is not a diagnosis, and the three crashes above
    // were all invisible precisely because the process left no statement of what had failed.
    let mut failures = 0;
    for (name, result) in [
        ("hook pipe", pipe),
        ("capture", capture_result),
        ("consolidation", consolidation),
        ("notes and projections", projections),
        ("source rediscovery", rediscovery),
        ("memory embedding", embedding),
    ] {
        if let Err(error) = result {
            failures += 1;
            tracing::error!(subsystem = name, %error, "subsystem stopped with an error");
        }
    }
    if failures > 0 {
        anyhow::bail!("{failures} subsystem(s) stopped with an error; see the log for each");
    }
    Ok(())
}

/// Run a subsystem, restarting it if it fails.
///
/// `join!` keeps one subsystem's failure from ending the others, but a subsystem that returned
/// is simply gone — capture would stay dead until someone noticed, which is quieter than a
/// crash and nearly as costly. Every one of these loops is idempotent on restart: cursors are
/// persisted, jobs are leased, and the projection is content-addressed, so re-entering one
/// resumes rather than repeats.
///
/// Backoff is capped so a subsystem failing on every attempt logs steadily instead of spinning
/// a core, and a clean return (shutdown) ends the supervision rather than restarting into a
/// service that is trying to stop.
async fn supervise<F, Fut>(
    name: &'static str,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    mut make: F,
) -> anyhow::Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut restarts: u32 = 0;
    loop {
        match make().await {
            Ok(()) => return Ok(()),
            Err(error) => {
                if *shutdown.borrow() {
                    // Failing while stopping is not worth restarting for.
                    tracing::warn!(subsystem = name, %error, "subsystem failed during shutdown");
                    return Ok(());
                }
                restarts += 1;
                let backoff = std::cmp::min(30, 1_u64 << std::cmp::min(restarts, 5));
                tracing::error!(
                    subsystem = name,
                    restarts,
                    backoff_seconds = backoff,
                    %error,
                    "subsystem failed; restarting"
                );
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(backoff)) => {}
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
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
