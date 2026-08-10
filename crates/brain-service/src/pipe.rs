use std::future::Future;

use anyhow::{Context, Result, bail};
use brain_domain::{
    HOOK_PROTOCOL_VERSION, HookEnvelope, decode_hook_frame_length, decode_hook_frame_payload,
    encode_hook_frame,
};

use crate::hook_handler::HookOutcome;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// How many pipe instances may exist at once.
///
/// This is the number of hooks that can be *in flight* together, and it used to be 2 — one being
/// served, one queued. That is enough only while every request is fast. It is not: the first
/// orientation after a service restart was measured at 6,571 ms against a cold cache, and with a
/// queue of one every other session start in that window failed too.
///
/// 16 is chosen against the burst this actually sees — a handful of harness windows resuming at
/// once, each firing `SessionStart` plus `UserPromptSubmit` — not against a load figure. Instances
/// are cheap; the cost of being one short is a session with no orientation.
const MAX_PIPE_INSTANCES: usize = 16;

pub struct HookPipeServer {
    pipe_name: String,
}

impl HookPipeServer {
    pub fn new(pipe_name: impl Into<String>) -> Self {
        Self {
            pipe_name: pipe_name.into(),
        }
    }

    pub async fn serve_once<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: FnOnce(HookEnvelope) -> Fut,
        Fut: Future<Output = Result<HookOutcome>>,
    {
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(1)
            .create(&self.pipe_name)
            .with_context(|| format!("create named pipe {}", self.pipe_name))?;
        server.connect().await.context("accept hook pipe client")?;
        handle_connected(&mut server, handler).await
    }

    pub async fn run<F, Fut>(
        &self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(HookEnvelope) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<HookOutcome>> + Send + 'static,
    {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(MAX_PIPE_INSTANCES);
        let mut server = options
            .create(&self.pipe_name)
            .with_context(|| format!("create first named pipe {}", self.pipe_name))?;
        options.first_pipe_instance(false);

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                connected = server.connect() => {
                    // Nothing in this arm may propagate. The whole service runs under one
                    // `try_join!`, so an error escaping here does not stop hook delivery — it
                    // stops capture, consolidation, rediscovery and projections too, and the
                    // process exits 1. A transient accept failure after thousands of
                    // connections is not a reason to lose the brain.
                    if let Err(error) = connected {
                        tracing::warn!(%error, "accept hook pipe client failed");
                    } else {
                        // Each request is served on its own task, and the accept loop goes
                        // straight back to waiting.
                        //
                        // Creating the next instance up front was already here, so a second client
                        // could *connect* mid-request — but this loop then awaited the handler, so
                        // nobody read that client's request until the first one finished. A queued
                        // client and an unserved one are the same thing from the far side of the
                        // pipe, and the client gives up after `HOOK_HARD_TIMEOUT`.
                        //
                        // That is how one slow request became three failures on 10 August: a
                        // cold-cache `SessionStart` took 6,571 ms, and the two hooks behind it
                        // timed out having done nothing wrong. Their replies were then written to
                        // pipes whose clients had already gone — the `write hook reply` warnings.
                        let next = create_listener(&options, &self.pipe_name).await;
                        let mut serving = std::mem::replace(&mut server, next);
                        let handler = handler.clone();
                        tokio::spawn(async move {
                            if let Err(error) = handle_connected(&mut serving, handler).await {
                                tracing::warn!(%error, "hook pipe request failed");
                            }
                        });
                        continue;
                    }
                    // The accept failed, so this instance is in an unknown state. Replace it
                    // rather than spinning on a listener that may never accept again.
                    server = create_listener(&options, &self.pipe_name).await;
                }
            }
        }
    }
}

/// Create a fresh pipe listener, retrying until one exists.
///
/// Returning an error instead would end the service, and a brain that cannot currently accept a
/// hook is in far better shape than one that has stopped capturing. The backoff is capped so a
/// prolonged failure logs steadily rather than spinning a core.
async fn create_listener(options: &ServerOptions, pipe_name: &str) -> NamedPipeServer {
    let mut attempt: u32 = 0;
    loop {
        match options.create(pipe_name) {
            Ok(server) => {
                if attempt > 0 {
                    tracing::info!(attempt, pipe_name, "hook pipe listener restored");
                }
                return server;
            }
            Err(error) => {
                attempt += 1;
                // Log the first failure and then sparingly; a per-attempt line would bury the
                // rest of the log during an outage.
                if attempt == 1 || attempt.is_multiple_of(20) {
                    tracing::error!(%error, attempt, pipe_name, "cannot create hook pipe listener");
                }
                let backoff = std::cmp::min(2_000, 50 * u64::from(attempt));
                tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            }
        }
    }
}

async fn handle_connected<F, Fut>(server: &mut NamedPipeServer, handler: F) -> Result<()>
where
    F: FnOnce(HookEnvelope) -> Fut,
    Fut: Future<Output = Result<HookOutcome>>,
{
    let envelope = read_envelope(server).await?;
    if envelope.protocol != HOOK_PROTOCOL_VERSION {
        bail!(
            "unsupported hook protocol {}; expected {}",
            envelope.protocol,
            HOOK_PROTOCOL_VERSION
        );
    }
    // Every hook that reaches us, logged before anything decides what to do with it.
    //
    // This is the instrument that was missing for the whole Codex investigation. Only
    // `SessionStart` leaves a delivery row; `UserPromptSubmit` records one solely when it has
    // something to push, and `SessionEnd` usually pushes nothing at all — so for two of the three
    // hooks, "the harness never called it" and "it called and we returned nothing" produce byte-for-byte
    // identical evidence. That ambiguity has now produced three wrong conclusions: that Codex
    // Desktop does not implement hooks, that `openai/codex#21639` was responsible, and most
    // recently that `UserPromptSubmit` had stopped firing.
    //
    // One line ends the whole class of question. It is `info` because the moment anybody needs it,
    // they need it about something that already happened.
    tracing::info!(
        harness = %envelope.harness.as_str(),
        event = %envelope.event_name,
        session = envelope
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-"),
        "hook received"
    );
    let outcome = handler(envelope).await?;
    let frame = encode_hook_frame(&outcome.reply)?;
    server.write_all(&frame).await.context("write hook reply")?;
    server.flush().await.context("flush hook reply")?;

    // Past this line the client has the orientation, and only now is it a delivery. Both `?`
    // above are the reason this cannot move any earlier: a write that fails returns here, and
    // for as long as the metric was recorded inside the handler those failures were counted as
    // successes anyway.
    if let Some(pending) = outcome.delivery {
        // Fail-open, and off the reactor. Recording opens SQLite, which is blocking work, and
        // the orientation has already been delivered — a metric that cannot be written is worth
        // a warning and nothing more.
        let recorded = tokio::task::spawn_blocking(move || pending.record()).await;
        match recorded {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "could not record a delivered orientation"),
            Err(join_error) => {
                tracing::warn!(%join_error, "delivery recording task failed")
            }
        }
    }
    Ok(())
}

async fn read_envelope(server: &mut NamedPipeServer) -> Result<HookEnvelope> {
    let mut prefix = [0_u8; 4];
    server
        .read_exact(&mut prefix)
        .await
        .context("read hook request length")?;
    let length = decode_hook_frame_length(prefix)?;
    let mut payload = vec![0_u8; length];
    server
        .read_exact(&mut payload)
        .await
        .context("read hook request payload")?;
    decode_hook_frame_payload(&payload)
}
