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
                    connected.context("accept hook pipe client")?;
                    let next = options
                        .create(&self.pipe_name)
                        .with_context(|| format!("create next named pipe {}", self.pipe_name))?;
                    if let Err(error) = handle_connected(&mut server, &handler).await {
                        tracing::warn!(%error, "hook pipe request failed");
                    }
                    server = next;
                }
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
