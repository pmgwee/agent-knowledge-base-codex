use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

use crate::CaptureSupervisor;

pub(crate) async fn run(
    supervisor: Arc<CaptureSupervisor>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let _ = watch_tx.send(result);
    })?;
    let parents = supervisor
        .watched_paths()
        .into_iter()
        .filter_map(|path| path.parent().map(PathBuf::from))
        .collect::<HashSet<_>>();
    // A source directory that has since been removed must not stop the service.
    //
    // Sources are recorded once and never withdrawn — cursors are keyed by source, so removing
    // an entry would orphan its cursor and re-ingest captured evidence. Meanwhile the agents
    // clean up their own session directories, so recorded paths go missing as a matter of
    // course, and rediscovery records hundreds of them.
    //
    // `watch()` on a vanished directory returns "Input watch path is neither a file nor a
    // directory". Propagated, that ends the whole service — the entire process runs under one
    // `try_join!` — and it happens during startup, before the logger has recorded anything, so
    // the only trace is Task Scheduler reporting exit code 1 and a log that simply stops. Every
    // session started while the brain is down loses its orientation, silently.
    //
    // The evidence for those sources is already captured. A path that cannot be watched only
    // means nothing new will arrive there, which for a finished session is true anyway.
    let mut watched = 0_usize;
    let mut unavailable = Vec::new();
    for parent in parents {
        match watcher.watch(&parent, RecursiveMode::NonRecursive) {
            Ok(()) => watched += 1,
            Err(error) => unavailable.push(format!("{}: {error}", parent.display())),
        }
    }
    if !unavailable.is_empty() {
        tracing::warn!(
            watched,
            unavailable = unavailable.len(),
            examples = ?unavailable.iter().take(3).collect::<Vec<_>>(),
            "some source directories could not be watched; capture continues for the rest"
        );
    }

    capture_without_stopping(&supervisor).await;
    let config = supervisor.config();
    let mut reconciliation = tokio::time::interval(config.reconciliation_interval);
    reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reconciliation.tick().await;

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = reconciliation.tick() => {
                capture_without_stopping(&supervisor).await;
            }
            event = watch_rx.recv() => {
                match event {
                    Some(Ok(_)) => {
                        tokio::time::sleep(config.watcher_debounce).await;
                        capture_without_stopping(&supervisor).await;
                    }
                    Some(Err(error)) => tracing::warn!(%error, "source watcher error"),
                    None => tracing::warn!("source watcher channel closed"),
                }
            }
        }
    }
}

async fn capture_without_stopping(supervisor: &CaptureSupervisor) {
    if let Err(error) = supervisor.capture_once().await {
        tracing::error!(%error, "capture reconciliation failed");
    }
}
