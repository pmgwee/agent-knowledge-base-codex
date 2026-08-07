use std::future::Future;

use anyhow::{Context, Result, bail};
use brain_domain::{
    HOOK_PROTOCOL_VERSION, HookEnvelope, HookReply, decode_hook_frame_length,
    decode_hook_frame_payload, encode_hook_frame,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

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
        Fut: Future<Output = Result<HookReply>>,
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
        F: Fn(HookEnvelope) -> Fut,
        Fut: Future<Output = Result<HookReply>>,
    {
        if *shutdown.borrow() {
            return Ok(());
        }
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(2);
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
                        // The next instance is created before serving this one, so a second
                        // client arriving mid-request is queued rather than refused.
                        let next = create_listener(&options, &self.pipe_name).await;
                        if let Err(error) = handle_connected(&mut server, &handler).await {
                            tracing::warn!(%error, "hook pipe request failed");
                        }
                        server = next;
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
    Fut: Future<Output = Result<HookReply>>,
{
    let envelope = read_envelope(server).await?;
    if envelope.protocol != HOOK_PROTOCOL_VERSION {
        bail!(
            "unsupported hook protocol {}; expected {}",
            envelope.protocol,
            HOOK_PROTOCOL_VERSION
        );
    }
    let reply = handler(envelope).await?;
    let frame = encode_hook_frame(&reply)?;
    server.write_all(&frame).await.context("write hook reply")?;
    server.flush().await.context("flush hook reply")?;
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
