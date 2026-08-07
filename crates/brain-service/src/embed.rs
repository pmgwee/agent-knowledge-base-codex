//! Embedding memories in the background.
//!
//! Consolidation produces memories; this gives each one a vector so retrieval can match meaning
//! as well as words. It is a backfill and a steady state at once — the same loop drains the
//! ~6,800 memories that already exist and picks up each new one as it is written.
//!
//! Two properties matter more than speed here.
//!
//! **It never blocks the runtime.** Embedding costs ~80 ms of pure CPU per memory, and every
//! subsystem in this service shares one tokio runtime. Running that on a worker thread starved
//! the reactor once already, in a different loop: a provider request timed out at exactly 60 s
//! against a connection that had opened in two, and it looked like a network fault for an hour.
//! The work goes to `spawn_blocking`.
//!
//! **It is entirely optional.** A brain with no model installed runs this loop as a no-op and
//! searches by keyword exactly as before. Nothing here may degrade retrieval that already works.

use std::path::PathBuf;

use anyhow::Result;
use brain_store::{Embedder, EventLedger, shared_embedder};

use crate::ServiceLaunchConfig;

/// Memories embedded per project per pass.
///
/// At ~11 embeddings a second a pass of 64 takes about six seconds, which is long enough to
/// make real progress and short enough that a shutdown does not wait on it. The bound also
/// keeps each pass's ledger handle short-lived rather than held for a whole backfill.
const BATCH: usize = 64;

/// Delay between passes once everything is embedded.
///
/// New memories arrive at consolidation's pace, so there is nothing to gain from spinning. When
/// a pass finds work it comes straight back rather than waiting.
const IDLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Delay between passes while a backlog is draining.
const BUSY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

pub async fn run_embedding_backfill(
    config: ServiceLaunchConfig,
    brain_home: PathBuf,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    pressure: tokio::sync::watch::Receiver<crate::DegradationState>,
) -> Result<()> {
    // Loading the model is itself ~80 ms of blocking work, and its absence is the common case
    // on a brain that never installed one.
    let home = brain_home.clone();
    let available = tokio::task::spawn_blocking(move || shared_embedder(&home).is_some()).await?;
    if !available {
        tracing::info!(
            "no embedding model installed; retrieval stays keyword-only and this loop is idle"
        );
        while shutdown.changed().await.is_ok() {
            if *shutdown.borrow() {
                break;
            }
        }
        return Ok(());
    }
    tracing::info!("embedding model loaded; backfilling memory vectors");

    let mut delay = BUSY_INTERVAL;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = tokio::time::sleep(delay) => {
                // Embedding is the most deferrable work in the service. Under disk pressure the
                // projection is already paused; joining it costs nothing a user would notice.
                if pressure.borrow().markdown_projection_paused {
                    delay = IDLE_INTERVAL;
                    continue;
                }

                let mut embedded_any = false;
                for project in &config.projects {
                    match embed_one_pass(project, &brain_home).await {
                        Ok(0) => {}
                        Ok(count) => {
                            embedded_any = true;
                            tracing::debug!(
                                project_id = %project.project_id.0,
                                count,
                                "embedded memory vectors"
                            );
                        }
                        Err(error) => tracing::warn!(
                            project_id = %project.project_id.0,
                            %error,
                            "embedding pass degraded for this project"
                        ),
                    }
                }
                // Straight back to work while a backlog remains; idle once it is drained.
                delay = if embedded_any { BUSY_INTERVAL } else { IDLE_INTERVAL };
            }
        }
    }
}

/// Embed up to `BATCH` documents for one project. Returns how many were written.
///
/// Memories are drained before events, and that ordering is the point rather than an accident.
/// There are a few thousand memories and a hundred thousand events, so memories-first means the
/// distilled layer is fully searchable by meaning within minutes while the raw layer fills in
/// over hours. A pass that interleaved them would leave both half-covered for the whole backfill.
async fn embed_one_pass(
    project: &crate::ServiceProjectConfig,
    brain_home: &std::path::Path,
) -> Result<usize> {
    let ledger_path = project.ledger_path.clone();
    let project_id = project.project_id;
    let home = brain_home.to_path_buf();

    // The whole pass — SQLite reads, model inference, SQLite writes — is blocking work. Sending
    // it to a blocking thread wholesale is simpler than interleaving, and correct for the same
    // reason: none of it may run on a reactor thread.
    tokio::task::spawn_blocking(move || -> Result<usize> {
        let Some(embedder) = shared_embedder(&home) else {
            return Ok(0);
        };
        let mut ledger = EventLedger::open(&ledger_path, project_id)?;
        let written = embed_memories(&mut ledger, embedder)?;
        if written > 0 {
            return Ok(written);
        }
        embed_events(&mut ledger, embedder)
    })
    .await?
}

fn embed_memories(ledger: &mut EventLedger, embedder: &Embedder) -> Result<usize> {
    let pending = ledger.memories_awaiting_embedding(BATCH)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let texts: Vec<&str> = pending.iter().map(|item| item.text.as_str()).collect();
    let vectors = embed_batch_checked(embedder, &texts)?;

    let now = time::OffsetDateTime::now_utc();
    let mut written = 0;
    for (item, vector) in pending.iter().zip(vectors) {
        // One memory failing to store is not a reason to discard the rest of the batch; the next
        // pass will offer it again.
        match ledger.store_memory_embedding(item, &vector, now) {
            Ok(()) => written += 1,
            Err(error) => tracing::warn!(
                memory = %item.memory_id,
                %error,
                "could not store a memory vector"
            ),
        }
    }
    Ok(written)
}

fn embed_events(ledger: &mut EventLedger, embedder: &Embedder) -> Result<usize> {
    let pending = ledger.events_awaiting_embedding(BATCH)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let now = time::OffsetDateTime::now_utc();

    // Most of what a harness records is not prose — an empty tool result, a bare path, a status
    // word. Those are marked considered and skipped rather than left pending, because a row this
    // pass declines but does not record is a row every future pass will fetch again, and a
    // backfill whose "what is left" count never reaches zero cannot be told from a stuck one.
    let (embeddable, skipped): (Vec<_>, Vec<_>) = pending
        .into_iter()
        .partition(|item| EventLedger::is_embeddable_event_text(&item.text));
    for item in &skipped {
        if let Err(error) = ledger.store_event_embedding(item.event_id, None, now) {
            tracing::warn!(event = %item.event_id, %error, "could not mark an event skipped");
        }
    }
    if embeddable.is_empty() {
        // Progress was still made: the skipped rows will not come back.
        return Ok(skipped.len());
    }

    let texts: Vec<&str> = embeddable.iter().map(|item| item.text.as_str()).collect();
    let vectors = embed_batch_checked(embedder, &texts)?;
    let mut written = skipped.len();
    for (item, vector) in embeddable.iter().zip(vectors) {
        match ledger.store_event_embedding(item.event_id, Some(&vector), now) {
            Ok(()) => written += 1,
            Err(error) => tracing::warn!(
                event = %item.event_id,
                %error,
                "could not store an event vector"
            ),
        }
    }
    Ok(written)
}

fn embed_batch_checked(embedder: &Embedder, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let vectors = embedder.embed_batch(texts)?;
    anyhow::ensure!(
        vectors.len() == texts.len(),
        "embedder returned {} vectors for {} inputs",
        vectors.len(),
        texts.len()
    );
    Ok(vectors)
}
