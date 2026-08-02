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
        .filter_map(|path| path.parent().map(PathBuf::from))
        .collect::<HashSet<_>>();
    for parent in parents {
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;
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
